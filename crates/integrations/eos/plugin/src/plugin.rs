//! Bevy plugin that ticks the EOS platform, drives the Connect and anti-cheat
//! lifecycle, and starts a `ClientServer` anti-cheat session.
//!
//! ## Native EOS login flow
//!
//! The host supplies an external credential. Connect returns a
//! `ProductUserId`, which the plugin uses to start anti-cheat.
//!
//! 1. `EOS_Connect_Login(EOS_ECT_OPENID_ACCESS_TOKEN, eos_access_token)`
//!    → `EOS_ProductUserId`
//! 2. `EOS_Connect_CreateUser(continuance)` if Connect returns
//!    `EOS_InvalidUser`
//! 3. `EOS_AntiCheatClient_BeginSession(LocalUserId = puid,
//!    Mode = ClientServer)` registers the outbound-message callback.
//!
//! The plugin keeps platform ticks and callbacks on the platform owner thread.

use crate::anticheat::AntiCheatClientState;
use crate::error::EosError;
use crate::service::{
    ConnectCreateUserPending, ConnectLoginOutcome, ConnectLoginPending, EosService,
};
use crate::settings::EosSettings;
use bevy::prelude::*;
use eos_ffi::{EOS_EExternalCredentialType, EOS_ProductUserId};
use std::fmt;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};
use zeroize::Zeroize;

const ANTI_CHEAT_RETRY_INTERVAL: Duration = Duration::from_secs(1);

/// Bevy plugin for EOS SDK integration.
pub struct EosPlugin;

/// Marker inserted only after the native-equivalent EOS anti-cheat session is
/// active.
///
/// `GridMate` connection creation must wait for this resource so the server
/// cannot deliver EOS anti-cheat traffic before `BeginSession`.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct EosAntiCheatReady;

/// External credential used by `EOS_Connect_Login`.
#[derive(Resource, Clone)]
pub struct EosConnectCredential {
    credential_type: EOS_EExternalCredentialType::Type,
    token: String,
}

impl EosConnectCredential {
    #[must_use]
    pub fn new(
        credential_type: EOS_EExternalCredentialType::Type,
        token: impl Into<String>,
    ) -> Self {
        Self {
            credential_type,
            token: token.into(),
        }
    }

    #[must_use]
    pub fn open_id(token: impl Into<String>) -> Self {
        Self::new(
            EOS_EExternalCredentialType::EOS_ECT_OPENID_ACCESS_TOKEN,
            token,
        )
    }
}

impl fmt::Debug for EosConnectCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EosConnectCredential")
            .field("credential_type", &self.credential_type)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

impl Drop for EosConnectCredential {
    fn drop(&mut self) {
        self.token.zeroize();
    }
}

impl Plugin for EosPlugin {
    fn build(&self, app: &mut App) {
        logging_registry::register!();
        app.init_non_send::<EosLoginState>();
        app.init_non_send::<AntiCheatClientState>();
        app.add_systems(Startup, init_eos_service);
        app.add_systems(
            Update,
            (tick_eos_platform, drive_eos_login.after(tick_eos_platform)),
        );
    }
}

fn init_eos_service(world: &mut World) {
    let settings = match world
        .get_resource::<EosSettings>()
        .cloned()
        .map_or_else(EosSettings::from_env, Ok)
    {
        Ok(settings) => settings,
        Err(e) => {
            error!("Failed to resolve EOS settings: {}", e);
            eprintln!("Warning: EOS features will be unavailable");
            return;
        }
    };

    match EosService::new(&settings) {
        Ok(service) => {
            world.insert_non_send(service);
            info!("EOS plugin initialized successfully");
        }
        Err(e) => {
            error!("Failed to initialize EOS plugin: {}", e);
            eprintln!("Warning: EOS features will be unavailable");
        }
    }
}

fn tick_eos_platform(eos: Option<NonSend<EosService>>) {
    if let Some(eos) = eos {
        eos.tick();
    }
}

#[derive(Default)]
enum EosLoginState {
    #[default]
    Idle,
    ConnectLoggingIn(ConnectLoginPending),
    CreatingUser(ConnectCreateUserPending),
    WaitingForAntiCheat {
        puid: EOS_ProductUserId,
        next_retry_at: Instant,
    },
    Active,
    Failed,
}

fn drive_eos_login(
    mut commands: Commands,
    state: Option<NonSendMut<EosLoginState>>,
    eos: Option<NonSend<EosService>>,
    credential: Option<Res<EosConnectCredential>>,
    anti_cheat: Option<NonSendMut<AntiCheatClientState>>,
) {
    let (Some(mut state), Some(eos), Some(credential), Some(mut anti_cheat)) =
        (state, eos, credential, anti_cheat)
    else {
        return;
    };

    if matches!(*state, EosLoginState::Active) && !anti_cheat.is_active() {
        commands.remove_resource::<EosAntiCheatReady>();
        *state = EosLoginState::Idle;
    }

    if matches!(*state, EosLoginState::Idle) {
        if let Some(next) = dispatch_connect_login(&eos, &credential) {
            *state = next;
        }
        return;
    }

    if let Some(next) = advance_login(&mut commands, &eos, &mut anti_cheat, &mut state) {
        *state = next;
    }
}

/// Dispatch `EOS_Connect_Login` after the host supplies a non-empty credential.
fn dispatch_connect_login(
    eos: &EosService,
    credential: &EosConnectCredential,
) -> Option<EosLoginState> {
    let token = credential.token.trim();
    if token.is_empty() {
        return None;
    }
    match eos.begin_connect_login(credential.credential_type, token) {
        Ok(pending) => {
            info!(
                "EOS_Connect_Login: dispatched (credential_type={:?}, token_len={})",
                credential.credential_type,
                token.len()
            );
            Some(EosLoginState::ConnectLoggingIn(pending))
        }
        Err(e) => {
            error!("EOS_Connect_Login dispatch failed: {:?}", e);
            Some(EosLoginState::Failed)
        }
    }
}

/// Poll whichever SDK request is in flight and return the state to move to, or
/// `None` to stay put for another frame.
fn advance_login(
    commands: &mut Commands,
    eos: &EosService,
    anti_cheat: &mut AntiCheatClientState,
    state: &mut EosLoginState,
) -> Option<EosLoginState> {
    match state {
        EosLoginState::ConnectLoggingIn(pending) => match pending.rx.try_recv() {
            Ok(Ok(ConnectLoginOutcome::Success(puid))) => {
                info!("EOS_Connect_Login: success");
                eos.set_local_user_id(puid);
                Some(start_anti_cheat(commands, eos, anti_cheat, puid))
            }
            Ok(Ok(ConnectLoginOutcome::NeedsCreateUser(continuance))) => {
                info!("EOS_Connect_Login: InvalidUser → CreateUser");
                match eos.begin_connect_create_user(continuance) {
                    Ok(p) => Some(EosLoginState::CreatingUser(p)),
                    Err(e) => {
                        error!("EOS_Connect_CreateUser dispatch failed: {:?}", e);
                        Some(EosLoginState::Failed)
                    }
                }
            }
            Ok(Err(e)) => {
                error!("EOS_Connect_Login: {:?}", e);
                Some(EosLoginState::Failed)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                error!("EOS_Connect_Login: callback channel disconnected");
                Some(EosLoginState::Failed)
            }
        },
        EosLoginState::CreatingUser(pending) => match pending.rx.try_recv() {
            Ok(Ok(puid)) => {
                info!("EOS_Connect_CreateUser: success");
                eos.set_local_user_id(puid);
                Some(start_anti_cheat(commands, eos, anti_cheat, puid))
            }
            Ok(Err(e)) => {
                error!("EOS_Connect_CreateUser: {:?}", e);
                Some(EosLoginState::Failed)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                error!("EOS_Connect_CreateUser: callback channel disconnected");
                Some(EosLoginState::Failed)
            }
        },
        EosLoginState::WaitingForAntiCheat {
            puid,
            next_retry_at,
        } => {
            let now = Instant::now();
            if now < *next_retry_at {
                None
            } else {
                match try_start_anti_cheat(commands, eos, anti_cheat, *puid) {
                    Ok(()) => Some(EosLoginState::Active),
                    Err(EosError::AuthInterfaceUnavailable) => {
                        *next_retry_at = now + ANTI_CHEAT_RETRY_INTERVAL;
                        None
                    }
                    Err(e) => {
                        error!("EOS_AntiCheatClient_BeginSession failed: {:?}", e);
                        Some(EosLoginState::Failed)
                    }
                }
            }
        }
        EosLoginState::Idle | EosLoginState::Active | EosLoginState::Failed => None,
    }
}

fn start_anti_cheat(
    commands: &mut Commands,
    eos: &EosService,
    anti_cheat: &mut AntiCheatClientState,
    puid: EOS_ProductUserId,
) -> EosLoginState {
    match try_start_anti_cheat(commands, eos, anti_cheat, puid) {
        Ok(()) => EosLoginState::Active,
        Err(EosError::AuthInterfaceUnavailable) => {
            warn!("EOS anti-cheat client interface unavailable; waiting for protected launch");
            commands.remove_resource::<EosAntiCheatReady>();
            EosLoginState::WaitingForAntiCheat {
                puid,
                next_retry_at: Instant::now() + ANTI_CHEAT_RETRY_INTERVAL,
            }
        }
        Err(e) => {
            error!("EOS_AntiCheatClient_BeginSession failed: {:?}", e);
            EosLoginState::Failed
        }
    }
}

fn try_start_anti_cheat(
    commands: &mut Commands,
    eos: &EosService,
    anti_cheat: &mut AntiCheatClientState,
    puid: EOS_ProductUserId,
) -> Result<(), EosError> {
    if !anti_cheat.is_active() {
        anti_cheat.start(eos, puid)?;
    }
    commands.insert_resource(EosAntiCheatReady);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::EosConnectCredential;

    #[test]
    fn connect_credential_debug_redacts_the_token() {
        let credential = EosConnectCredential::open_id("credential-secret");

        let rendered = format!("{credential:?}");

        assert!(rendered.contains("credential_type"));
        assert!(rendered.contains("[REDACTED]"));
        assert!(!rendered.contains("credential-secret"));
    }
}
