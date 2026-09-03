use crate::az::{Class, MessageInfo, TypeRegistry};
use crate::serialize::{
    buffer::{CARRIER_ENDIAN, ReadBuffer, WriteBuffer},
    crc32::Crc32,
    error::MarshalerError,
    marshaler::Marshaler,
    vlq::VlqU32Marshaler,
};
use az_core::type_info::AzTypeInfo;
use std::{any::Any, fmt::Debug, marker::PhantomData};
use uuid::Uuid;

// ============================================================================
// GridMate Message Type System
// ============================================================================

/// UUID of the C++ `Amazon::Hub::IMessage` abstract base class itself —
/// the root type identifier that every `IMessage` subclass inherits from.
///
/// Exposed as a `const` rather than an [`AzTypeInfo`] impl
/// because `IMessage` itself has no wire shape — only its leaves do —
/// and we want a single grep-able anchor when chasing RTTI strings.
pub const IMESSAGE_BASE_UUID: Uuid = Uuid::from_u128(0x7F4B41C6_06AE_47A1_A144_5F8BA048B0EF);

/// Source `Amazon::Hub::IActor` placeholder accepted by `Message::execute`.
///
/// Actor execution is owned by higher-level game systems in this Rust port;
/// the hook is retained so `Message` covers the source virtual surface.
pub trait Actor: Any + Debug + Send + Sync {}

/// Top-level wire message -- a [`Class`] that additionally has an
/// Message envelope index for routing inbound packets.
///
/// # C++ counterpart
///
/// Each `impl Message for T` corresponds to a concrete C++ subclass of
/// `Amazon::Hub::IMessage`. The abstract base class's own UUID is
/// [`IMESSAGE_BASE_UUID`].
///
/// The Rust port covers the wire shape plus the source virtual hooks:
///
/// | C++ `IMessage` member | Rust mapping |
/// |---|---|
/// | `MF_RTTI(IMessage, "7F4B41C6-…")` | [`IMESSAGE_BASE_UUID`] constant + each leaf's [`AzTypeInfo::TYPE_ID`](az_core::type_info::AzTypeInfo::TYPE_ID) |
/// | per-subclass `MF_RTTI` macros (concrete UUIDs) | [`ClassDesc`](crate::ClassDesc) derive on the leaf struct |
/// | `virtual std::string ToString()` | [`Message::message_string`] |
/// | `virtual std::string ParamsToString()` | [`Message::params_to_string`] |
/// | `virtual bool Execute(IActor&)` | [`Message::execute`], defaulting to no-op |
/// | `static Create<MessageT>(args)` allocator factory | construct directly; Rust's stack allocation removes the pool-allocator concern |
///
/// # Hub extension to vanilla AZ
///
/// Hub messaging adds a sparse
/// numeric envelope-index (`type_index`) as a wire-compression optimisation
/// so each top-level packet header carries `VlqU32` instead of a 16-byte
/// UUID. The UUID side of identity comes from [`AzTypeInfo`].
///
/// # Implementing
///
/// Don't impl by hand. Use:
///
/// ```ignore
/// #[derive(Debug, Clone, Marshaler, ClassDesc, Message)]
/// #[class_desc(uuid = "6A379FB8-0BDD-43A1-AB3E-9843D7BE8CD3", type_index = 349)]
/// pub struct PingMsg { pub elapsed: gridmate::pervasives::Duration }
/// ```
///
/// `Debug` is a required part of the contract so any registered message can
/// participate in generic protocol diagnostics without per-consumer bounds.
pub trait Message: Class + Debug + Send + Sync {
    /// Message envelope index supplied by the Rust class descriptor.
    ///
    /// On the wire: appears as `VlqU32 type_index` in the envelope header.
    const TYPE_INDEX: u32 = <Self as Class>::TYPE_INDEX;

    /// Protocol metadata this message's registration carries.
    ///
    /// Lives on the trait so `ClassRegistration::of_message::<Self>()` reads it
    /// from the type: a registration site names the type and nothing else, and
    /// the `#[message(actor_scoped)]` declaration cannot drift away from the
    /// entry that publishes it.
    const INFO: MessageInfo = MessageInfo {
        actor_scoped: false,
    };

    /// Source `IMessage::ParamsToString`.
    fn params_to_string(&self) -> String {
        "...".to_owned()
    }

    /// Source `IMessage::ToString`.
    fn message_string(&self) -> String {
        format!(
            "{}({})",
            <Self as AzTypeInfo>::NAME,
            self.params_to_string()
        )
    }

    /// Source `IMessage::Execute`.
    fn execute(&self, _actor: &mut dyn Actor) -> bool {
        false
    }
}

/// Source-level direction markers for the Hub message wrapper.
pub mod path {
    #[derive(Debug, Clone, Copy, Default)]
    pub struct ClientToServer;

    #[derive(Debug, Clone, Copy, Default)]
    pub struct ServerToClient;
}

pub use path::{ClientToServer, ServerToClient};

/// Wire path marker.
///
/// The path determines which metadata wraps the [`MessageEnvelope`]; it is
/// intentionally separate from `T` because a few Hub messages, notably ping,
/// use the same message type in both directions.
pub trait MessagePath {
    type Metadata: Debug + Clone;
}

impl MessagePath for ClientToServer {
    type Metadata = CorrelatedMetadata;
}

impl MessagePath for ServerToClient {
    type Metadata = SizedEnvelopeMetadata;
}

/// An outbound [`MessagePath`] — knows how to encode a typed message
/// into wire bytes including its path-specific outer framing.
///
/// `Context` is the per-path "what the call site supplies" type:
/// `Uuid` for [`ClientToServer`] (the correlation slot carries the
/// target actor UUID here), `()` for [`ServerToClient`] (sized frame
/// has no outer header beyond the VLQ size). Implementations only
/// take what they actually consume — there's no `correlation_id`
/// param ignored at the `ServerToClient` impl.
pub trait EgressPath: MessagePath + Sized + 'static {
    /// Path-specific framing input supplied at the call site.
    type Context: Default + Send + 'static;

    fn marshal_framed<T: Message + Marshaler>(msg: T, context: Self::Context, wb: &mut WriteBuffer);
}

impl EgressPath for ClientToServer {
    type Context = Uuid;
    fn marshal_framed<T: Message + Marshaler>(msg: T, correlation_id: Uuid, wb: &mut WriteBuffer) {
        let mut meta = MessageMetadata::<T, Self>::with_correlation_id(msg, correlation_id);
        meta.marshal(wb);
    }
}

impl EgressPath for ServerToClient {
    type Context = ();
    fn marshal_framed<T: Message + Marshaler>(msg: T, (): (), wb: &mut WriteBuffer) {
        let mut meta = MessageMetadata::<T, Self>::new(msg);
        meta.marshal(wb);
    }
}

/// An inbound [`MessagePath`] — knows how to decode a typed message
/// from wire bytes including its path-specific outer framing.
///
/// Direction markers are source-level directions, not local endpoint
/// roles: [`ClientToServer`] reads/writes the correlated C→S frame,
/// and [`ServerToClient`] reads/writes the sized S→C frame.
pub trait IngressPath: MessagePath + Sized + 'static {
    /// Decode one typed message, including this path's outer framing.
    ///
    /// # Errors
    ///
    /// Returns [`MarshalerError::BufferUnderrun`] if `rb` ends inside the
    /// frame, [`MarshalerError::CrcMismatch`] or
    /// [`MarshalerError::TruncatedPayload`] if the frame's own header does not
    /// match its body, [`MarshalerError::MessageTypeMismatch`] if the envelope
    /// names a different message than `T`, and whatever `T`'s own decoder
    /// raises for the body.
    fn read_framed<T: Message + Marshaler>(
        rb: &mut ReadBuffer,
    ) -> Result<MessageMetadata<T, Self>, MarshalerError>;
}

impl IngressPath for ClientToServer {
    fn read_framed<T: Message + Marshaler>(
        rb: &mut ReadBuffer,
    ) -> Result<MessageMetadata<T, Self>, MarshalerError> {
        MessageMetadata::<T, Self>::unmarshal(rb)
    }
}

impl IngressPath for ServerToClient {
    fn read_framed<T: Message + Marshaler>(
        rb: &mut ReadBuffer,
    ) -> Result<MessageMetadata<T, Self>, MarshalerError> {
        MessageMetadata::<T, Self>::unmarshal(rb)
    }
}

/// A message that travels over wire egress path `P`.
///
/// Parameterised by the path rather than an associated type so a single
/// message type can implement [`Sendable`] for multiple directions — the
/// canonical case is `PingMsg`, which the server originates over
/// [`ServerToClient`] and the client echoes back over [`ClientToServer`].
///
/// Sinks fix the direction at the call site by either constraining
/// `P` to a specific path (e.g. a server-only listener that always
/// emits via [`ServerToClient`]) or by leaving `P` generic and letting
/// type inference pick from the message's implemented directions.
///
/// `#[derive(Message)]` can emit these impls from source declaration
/// direction (`#[message(client_to_server)]` / `#[message(server_to_client)]`).
/// Hand-written impls remain appropriate for bidirectional or exceptional
/// Hub messages.
pub trait Sendable<P: EgressPath>: Message + Marshaler + Send + 'static {}

/// A message that arrives over wire ingress path `P`.
///
/// Mirror of [`Sendable<P>`]: parameterised by the path so a single message
/// type can implement [`Receivable`] for both [`ClientToServer`] and
/// [`ServerToClient`] when the protocol uses the same shape in both
/// directions (`PingMsg`).
pub trait Receivable<P: IngressPath>: Message + Marshaler + Send + 'static {}

/// C->S Hub metadata: CRC/size/correlation followed by a `MessageEnvelope`.
///
/// Wire:
///
/// ```text
/// [crc32:u32-be]
/// [payload_size:u32-be = correlation_id(16) + MessageEnvelope.len]
/// [correlation_id:uuid]
/// [MessageEnvelope]
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorrelatedMetadata {
    pub checksum: u32,
    pub payload_size: u32,
    pub correlation_id: Uuid,
}

impl CorrelatedMetadata {
    #[inline]
    #[must_use]
    pub const fn new(correlation_id: Uuid) -> Self {
        Self {
            checksum: 0,
            payload_size: 0,
            correlation_id,
        }
    }
}

/// S->C Hub stream item metadata: VLQ envelope size followed by a
/// `MessageEnvelope`.
///
/// Wire:
///
/// ```text
/// [envelope_size:vlq32 = MessageEnvelope.len]
/// [MessageEnvelope]
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SizedEnvelopeMetadata {
    pub envelope_size: u32,
}

/// Message identity as encoded inside `Amazon::Hub::MessageEnvelope`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageTypeId {
    TypeIndex(u32),
    Uuid(Uuid),
}

impl MessageTypeId {
    #[inline]
    #[must_use]
    pub const fn type_index(self) -> Option<u32> {
        match self {
            Self::TypeIndex(type_index) => Some(type_index),
            Self::Uuid(_) => None,
        }
    }

    #[inline]
    fn matches<T: Message>(self) -> bool {
        match self {
            Self::TypeIndex(type_index) => type_index == <T as Message>::TYPE_INDEX,
            Self::Uuid(uuid) => uuid == <T as AzTypeInfo>::TYPE_ID,
        }
    }

    /// Resolve to the numeric typeIndex, mapping UUIDs through
    /// [`TypeRegistry::type_index_of`]. Returns `0` when an inbound UUID isn't
    /// registered by this host, and when no registry is bound at all — this is
    /// a label for diagnostics, so an unresolvable id degrades to `0` here
    /// instead of failing a decode.
    #[inline]
    #[must_use]
    pub fn display_index(self, types: Option<TypeRegistry<'_>>) -> u32 {
        match self {
            Self::TypeIndex(type_index) => type_index,
            Self::Uuid(uuid) => types
                .and_then(|types| types.type_index_of(&uuid))
                .unwrap_or(0),
        }
    }
}

/// Borrowed wire view of one native `Amazon::Hub::MessageEnvelope`.
///
/// This is intentionally not a second high-level envelope abstraction. It is
/// the dynamic-dispatch read path: receive/capture code has to read
/// `messageTypeId` before it can choose a concrete `MessageEnvelope<T>`.
#[derive(Debug, Clone, Copy)]
pub struct MessageEnvelopeView<'a> {
    pub raw: &'a [u8],
    pub outer_flags: u8,
    pub field1: Option<[u8; 8]>,
    pub field2: Option<[u8; 8]>,
    pub envelope_flags: u8,
    pub type_id: MessageTypeId,
    pub body: &'a [u8],
}

impl MessageEnvelopeView<'_> {
    #[inline]
    #[must_use]
    pub const fn type_index(&self) -> Option<u32> {
        self.type_id.type_index()
    }
}

/// Native Hub message envelope.
///
/// This owns only `Amazon::Hub::MessageEnvelope`:
///
/// ```text
/// [outer_flags:u8]
/// [field1:u64? if outer_flags & 0x01]
/// [field2:u64? if outer_flags & 0x02]
/// [envelope_flags:u8]
/// [messageTypeId:vlq32 or UUID if 0]
/// [m_message bytes]
/// ```
///
/// Directional CRC/size/correlation and S->C VLQ stream sizing live in
/// [`MessageMetadata`], not here.
#[derive(Debug, Clone)]
pub struct MessageEnvelope<T> {
    pub message: T,
}

impl<T: Message> MessageEnvelope<T> {
    #[inline]
    pub(crate) const fn new(message: T) -> Self {
        Self { message }
    }

    pub(crate) fn marshal(&self, wb: &mut WriteBuffer) {
        0u8.marshal(wb);
        1u8.marshal(wb);
        VlqU32Marshaler.marshal(wb, <T as Message>::TYPE_INDEX);
        self.message.marshal(wb);
    }
}

/// Path-specific wrapper around a [`MessageEnvelope`].
#[derive(Debug, Clone)]
pub struct MessageMetadata<T, P: MessagePath = ClientToServer> {
    pub metadata: P::Metadata,
    pub envelope: MessageEnvelope<T>,
    _path: PhantomData<P>,
}

impl<T, P> MessageMetadata<T, P>
where
    P: MessagePath,
{
    #[inline]
    const fn from_parts(metadata: P::Metadata, envelope: MessageEnvelope<T>) -> Self {
        Self {
            metadata,
            envelope,
            _path: PhantomData,
        }
    }

    #[inline]
    pub fn into_message(self) -> T {
        self.envelope.message
    }
}

impl<T: Message> MessageMetadata<T, ClientToServer> {
    pub const fn new(message: T) -> Self {
        Self::with_correlation_id(message, Uuid::nil())
    }

    pub const fn with_correlation_id(message: T, correlation_id: Uuid) -> Self {
        Self::from_parts(
            CorrelatedMetadata::new(correlation_id),
            MessageEnvelope::new(message),
        )
    }

    pub const fn set_correlation_id(&mut self, correlation_id: Uuid) {
        self.metadata.correlation_id = correlation_id;
    }

    pub fn marshal(&mut self, wb: &mut WriteBuffer) {
        marshal_correlated_into(&mut self.metadata, &self.envelope, wb);
    }

    /// Decode a C→S correlated frame carrying a `T`.
    ///
    /// # Errors
    ///
    /// Returns any error [`read_correlated_message`] raises for the outer
    /// frame — [`MarshalerError::CrcMismatch`],
    /// [`MarshalerError::TruncatedPayload`], [`MarshalerError::BufferUnderrun`]
    /// — or [`MarshalerError::MessageTypeMismatch`] if the envelope names a
    /// message other than `T`, plus whatever `T`'s decoder raises for the body.
    pub fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let types = rb.types();
        let (metadata, view) = read_correlated_message(rb)?;
        Ok(Self::from_parts(
            metadata,
            decode_typed_envelope(view, types)?,
        ))
    }
}

impl<T: Message> MessageMetadata<T, ServerToClient> {
    pub fn new(message: T) -> Self {
        Self::from_parts(
            SizedEnvelopeMetadata::default(),
            MessageEnvelope::new(message),
        )
    }

    pub fn marshal(&mut self, wb: &mut WriteBuffer) {
        marshal_sized_into(&mut self.metadata, &self.envelope, wb);
    }
    /// Decode an S→C sized frame carrying a `T`.
    ///
    /// # Errors
    ///
    /// Returns any error [`read_sized_message`] raises for the outer frame —
    /// [`MarshalerError::TruncatedPayload`] on a bad VLQ size,
    /// [`MarshalerError::BufferUnderrun`] on a short read — or
    /// [`MarshalerError::MessageTypeMismatch`] if the envelope names a message
    /// other than `T`, plus whatever `T`'s decoder raises for the body.
    pub fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let types = rb.types();
        let (metadata, view) = read_sized_message(rb)?;
        Ok(Self::from_parts(
            metadata,
            decode_typed_envelope(view, types)?,
        ))
    }
}

/// Stream one `Amazon::Hub::MessageEnvelope` from `rb`, consuming exactly
/// `envelope_len` bytes.
///
/// The returned [`MessageEnvelopeView`] borrows the consumed region; `rb` is
/// left positioned for follow-on reads.
///
/// `envelope_flags == 0` (intentionally-empty record) surfaces as
/// [`MarshalerError::EmptyEnvelope`] so multi-record loops can
/// match-and-continue without conflating "no body" with malformed input.
///
/// # Errors
///
/// Returns [`MarshalerError::EmptyEnvelope`] for `envelope_flags == 0`,
/// [`MarshalerError::InvalidEnvelopeFlags`] for any value `>= 2`, and
/// [`MarshalerError::BufferUnderrun`] if `rb` holds fewer than `envelope_len`
/// bytes or the envelope ends inside its own header — the outer flag fields,
/// the VLQ type index, or the 16 raw UUID bytes that follow a zero index.
pub fn read_message_envelope<'a>(
    rb: &mut ReadBuffer<'a>,
    envelope_len: usize,
) -> Result<MessageEnvelopeView<'a>, MarshalerError> {
    let envelope_bytes = rb.read_bytes(envelope_len)?;
    let mut local = ReadBuffer::new(CARRIER_ENDIAN, envelope_bytes);

    let outer_flags = local.read_u8()?;
    let field1 = if (outer_flags & 0x01) != 0 {
        Some(read_fixed_8(&mut local)?)
    } else {
        None
    };
    let field2 = if (outer_flags & 0x02) != 0 {
        Some(read_fixed_8(&mut local)?)
    } else {
        None
    };

    let envelope_flags = local.read_u8()?;
    if envelope_flags == 0 {
        return Err(MarshalerError::EmptyEnvelope);
    }
    if envelope_flags >= 2 {
        return Err(MarshalerError::InvalidEnvelopeFlags {
            flags: envelope_flags,
        });
    }

    let vlq_value = VlqU32Marshaler.unmarshal(&mut local)?;
    let type_id = if vlq_value == 0 {
        let uuid_bytes = local.read_bytes(16)?;
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(uuid_bytes);
        MessageTypeId::Uuid(Uuid::from_bytes(bytes))
    } else {
        MessageTypeId::TypeIndex(vlq_value)
    };

    let body = local.read_bytes(local.left())?;
    Ok(MessageEnvelopeView {
        raw: envelope_bytes,
        outer_flags,
        field1,
        field2,
        envelope_flags,
        type_id,
        body,
    })
}

/// Stream one C→S correlated message from `rb`: outer CRC32 / payload
/// size / correlation UUID, followed by an envelope. CRC is validated
/// against `correlation_id || envelope_bytes` before parsing.
///
/// # Errors
///
/// Returns [`MarshalerError::TruncatedPayload`] if `payload_size` is under the
/// 16-byte correlation id or overruns what `rb` still holds,
/// [`MarshalerError::CrcMismatch`] if the header checksum disagrees with
/// `correlation_id || envelope_bytes`, [`MarshalerError::BufferUnderrun`] if
/// `rb` ends inside the 24-byte header, and any error
/// [`read_message_envelope`] raises for the envelope itself.
pub fn read_correlated_message<'a>(
    rb: &mut ReadBuffer<'a>,
) -> Result<(CorrelatedMetadata, MessageEnvelopeView<'a>), MarshalerError> {
    let checksum = rb.read_u32()?;
    let payload_size = rb.read_u32()?;
    let correlation_id = Uuid::unmarshal(rb)?;

    let envelope_size = payload_size
        .checked_sub(16)
        .ok_or(MarshalerError::TruncatedPayload {
            declared: 16,
            available: payload_size as usize,
        })? as usize;
    if envelope_size > rb.left() {
        return Err(MarshalerError::TruncatedPayload {
            declared: envelope_size,
            available: rb.left(),
        });
    }

    // CRC covers `correlation_id || envelope_bytes`. Peek the slice (no
    // cursor advance) so we can validate, then stream-read the envelope.
    let envelope_bytes = &rb.remaining()[..envelope_size];
    let actual_crc = Crc32::new()
        .update(correlation_id.as_bytes())
        .update(envelope_bytes)
        .finalize();
    if actual_crc != checksum {
        return Err(MarshalerError::CrcMismatch {
            expected: checksum,
            actual: actual_crc,
        });
    }

    let view = read_message_envelope(rb, envelope_size)?;
    Ok((
        CorrelatedMetadata {
            checksum,
            payload_size,
            correlation_id,
        },
        view,
    ))
}

/// Stream one S→C sized message from `rb`: VLQ envelope size, followed
/// by an envelope. Caller is expected to gate this on `rb.is_empty()` —
/// exhausted buffers are a loop condition, not a parse outcome.
///
/// # Errors
///
/// Returns [`MarshalerError::TruncatedPayload`] if the VLQ envelope size is
/// zero or larger than what `rb` still holds,
/// [`MarshalerError::BufferUnderrun`] if `rb` ends inside that VLQ size, and
/// any error [`read_message_envelope`] raises for the envelope itself.
pub fn read_sized_message<'a>(
    rb: &mut ReadBuffer<'a>,
) -> Result<(SizedEnvelopeMetadata, MessageEnvelopeView<'a>), MarshalerError> {
    let envelope_size = VlqU32Marshaler.unmarshal(rb)?;
    if envelope_size == 0 || envelope_size as usize > rb.left() {
        return Err(MarshalerError::TruncatedPayload {
            declared: envelope_size as usize,
            available: rb.left(),
        });
    }

    let view = read_message_envelope(rb, envelope_size as usize)?;
    Ok((SizedEnvelopeMetadata { envelope_size }, view))
}

fn marshal_correlated_into<T: Message>(
    metadata: &mut CorrelatedMetadata,
    envelope: &MessageEnvelope<T>,
    wb: &mut WriteBuffer,
) {
    // 24-byte header: [crc32:u32][payload_size:u32][correlation_id:uuid].
    // `with_fixed_prefix` reserves 24 zero bytes, runs the envelope
    // marshal, then hands us back both the prefix region and the body
    // slice so we can compute CRC + size and patch them in place.
    wb.with_fixed_prefix(
        24,
        |wb| envelope.marshal(wb),
        |prefix, envelope_bytes| {
            // Carrier framing caps one message far below 4 GiB
            // (`MAX_NUM_CHUNKS * MAX_MESSAGE_DATA_SIZE`, about 36 MB), so the
            // fallback is unreachable; it exists so an impossible length cannot
            // silently wrap into a plausible one.
            metadata.payload_size = u32::try_from(16 + envelope_bytes.len()).unwrap_or(u32::MAX);
            metadata.checksum = Crc32::new()
                .update(metadata.correlation_id.as_bytes())
                .update(envelope_bytes)
                .finalize();
            prefix[0..4].copy_from_slice(&metadata.checksum.to_be_bytes());
            prefix[4..8].copy_from_slice(&metadata.payload_size.to_be_bytes());
            prefix[8..24].copy_from_slice(metadata.correlation_id.as_bytes());
        },
    );
}

fn marshal_sized_into<T: Message>(
    metadata: &mut SizedEnvelopeMetadata,
    envelope: &MessageEnvelope<T>,
    wb: &mut WriteBuffer,
) {
    let mut envelope_buf = WriteBuffer::new(CARRIER_ENDIAN);
    envelope.marshal(&mut envelope_buf);
    // Same 32-bit wire width as `marshal_correlated_into`: carrier framing caps
    // one message far below 4 GiB, so the fallback is unreachable.
    metadata.envelope_size = u32::try_from(envelope_buf.len()).unwrap_or(u32::MAX);
    VlqU32Marshaler.marshal(wb, metadata.envelope_size);
    wb.write_bytes(envelope_buf.as_slice());
}

fn decode_typed_envelope<'a, T: Message>(
    parsed: MessageEnvelopeView<'a>,
    types: Option<TypeRegistry<'a>>,
) -> Result<MessageEnvelope<T>, MarshalerError> {
    if !parsed.type_id.matches::<T>() {
        return Err(MarshalerError::MessageTypeMismatch {
            expected: <T as Message>::TYPE_INDEX,
            actual: parsed.type_id.display_index(types),
        });
    }

    let mut body_rb = ReadBuffer::new(CARRIER_ENDIAN, parsed.body);
    if let Some(types) = types {
        body_rb = body_rb.with_types(types);
    }
    let message = T::unmarshal(&mut body_rb)?;
    Ok(MessageEnvelope { message })
}

fn read_fixed_8(rb: &mut ReadBuffer) -> Result<[u8; 8], MarshalerError> {
    let bytes = rb.read_bytes(8)?;
    let mut out = [0u8; 8];
    out.copy_from_slice(bytes);
    Ok(out)
}
