//! Descriptor attribute for the Azoth typed gem contribution contract.
//!
//! `#[contribution]` sits on a contribution's `impl Contribution` block. In its
//! primary form it reads identity, target roles, and the host-capability floor
//! out of the gem's own `gem.toml` — with no arguments when the package hosts
//! one contribution, and with that contribution's id as a bare token when it
//! hosts several — so the whole hand-written registration surface of a gem is a
//! type and a `register` body. The explicit four-key form spells the same facts
//! at the call site for code that has no manifest to read.
//!
//! Either way the attribute generates the capability declaration through
//! `az_gem_contract::declare_caps!`, wires `type Caps` to it, and emits the
//! pure-data `descriptor()` accessor; `register` and the lifecycle phases stay
//! the author's own code inside the same block.

mod manifest;

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::meta::ParseNestedMeta;
use syn::spanned::Spanned;
use syn::{
    Expr, ExprArray, FnArg, Ident, ImplItem, ItemImpl, Lit, Pat, Path, Signature, Stmt, Type,
    parse_macro_input,
};

/// Declares a contribution's identity, roles, and capability floor.
///
/// # The zero-argument form
///
/// ```ignore
/// struct Runtime;
///
/// #[contribution]
/// impl Contribution for Runtime {
///     fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) { /* … */ }
/// }
/// ```
///
/// The attribute walks up from `CARGO_MANIFEST_DIR` to the nearest `gem.toml`
/// and selects the `[[contributions]]` entry whose `package` is this crate's
/// `CARGO_PKG_NAME`. That entry's `id`, `roles`, and `caps` become the
/// descriptor and the capability floor, and the entry item generated glue calls
/// — `<id>_contribution()`, with `-` and `.` folded to `_` — is emitted here,
/// so the naming convention is macro-internal and no author writes either side
/// of it. The marked type stays private: sealing is privacy, and the entry item
/// returns `impl Contribution`.
///
/// Activation is not part of it: what the project selected is per-instance
/// composition data, so it arrives at `Composer::add` beside the contribution
/// and is read back through `GemContext::activation`, never threaded through
/// the entry item.
///
/// # The bare-token form
///
/// ```ignore
/// #[contribution(runtime)]
/// impl Contribution for Runtime { /* … */ }
///
/// #[contribution(runtime_host)]
/// impl Contribution for Host { /* … */ }
/// ```
///
/// One package may implement several of its gem's contributions, and then the
/// package alone does not say which block is which. The token names it: an
/// ident matched against the ids this package declares, with `-` and `.`
/// written as `_` on either side. It is validated against the same manifest
/// that answers the zero-argument form — a token nothing declares is refused
/// with the package's own ids listed, and a token naming another package's
/// contribution is refused naming that package — so the token narrows the
/// search and never becomes a second place identity is spelled.
///
/// Either manifest form reads `engine.toml` where the crate lives under the
/// engine rather than inside a gem: the engine is a manifested composition root
/// with the same stanza grammar, so an engine author writes a bare
/// `#[contribution]` and no string, exactly as a gem author does.
///
/// The expansion also `include_bytes!`es the manifest under an anonymous const,
/// which puts `gem.toml` in the crate's dep-info: editing the manifest rebuilds
/// the crate rather than leaving a stale descriptor compiled in.
///
/// # Generated products
///
/// A stanza's `products = ["graphs"]` says the crate's build script compiled a
/// generated projection in. The expansion registers it as the *first* statement
/// of the `register` body, so the declaration is the whole of the author's
/// part: no registration line to write, none to forget, and a declared product
/// the build never emitted is a compile error in this crate rather than a
/// runtime missing its graphs.
///
/// ## Why an id literal appears here
///
/// The zero-argument expansion constructs `GemId::new("…")` and
/// `ContributionId::new("…")` from manifest-read values. That is the one
/// sanctioned place a literal is minted: it is machine-derived from the single
/// source of identity, const-validated against the dotted-id grammar at compile
/// time, and never typed by a human. The gem's generated ids crate stays the
/// surface for *cross-gem* references — a gem naming another gem's contribution
/// still consumes those consts, because the other gem's manifest is not this
/// crate's to read.
///
/// # The explicit form
///
/// ```ignore
/// #[contribution(
///     gem = vegetation_ids::GEM,
///     id = vegetation_ids::contributions::PACKAGE,
///     roles = [Server, Unified, HeadlessServer],
///     caps = [HasAuthority, HostsWorld],
/// )]
/// impl Contribution for RuntimeServer { /* … */ }
/// ```
///
/// All four keys are required together, ids are consts rather than literals,
/// and no entry item is generated: the id arrives as an opaque const expression
/// the macro cannot read, so it cannot name the function. Tests and spikes that
/// live outside any gem use this form. Keys and a bare token are two answers to
/// the same question, so the attribute takes one or the other, never both.
#[proc_macro_attribute]
pub fn contribution(args: TokenStream, item: TokenStream) -> TokenStream {
    let mut declaration = Declaration::default();
    let parser = syn::meta::parser(|meta| declaration.parse_entry(&meta));
    parse_macro_input!(args with parser);
    let block = parse_macro_input!(item as ItemImpl);
    expand(&declaration, &block)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[derive(Default)]
struct Declaration {
    /// The bare token, when the attribute was given one.
    token: Option<Ident>,
    gem: Option<Expr>,
    id: Option<Expr>,
    roles: Option<Vec<proc_macro2::TokenStream>>,
    caps: Option<Vec<Ident>>,
}

/// The shape `caps = [...]` takes, quoted back on every malformed spelling.
const CAPS_SHAPE: &str =
    "`caps` takes host-capability tokens, for example `caps = [HasAuthority, HostsWorld]`";

impl Declaration {
    fn parse_entry(&mut self, meta: &ParseNestedMeta<'_>) -> syn::Result<()> {
        if !meta.input.peek(syn::Token![=]) {
            return self.name(meta);
        }
        if meta.path.is_ident("gem") {
            replace_once(&mut self.gem, id_const(meta)?, meta)
        } else if meta.path.is_ident("id") {
            replace_once(&mut self.id, id_const(meta)?, meta)
        } else if meta.path.is_ident("roles") {
            let paths = path_array(
                meta,
                "`roles` takes `GemTargetRole` variants, for example `roles = [Server, Unified]`",
            )?;
            replace_once(&mut self.roles, role_tokens(paths), meta)
        } else if meta.path.is_ident("caps") {
            let paths = path_array(meta, CAPS_SHAPE)?;
            replace_once(&mut self.caps, cap_idents(paths)?, meta)
        } else {
            Err(meta
                .error("unsupported `contribution` key; expected `gem`, `id`, `roles`, or `caps`"))
        }
    }

    /// A bare token: the id of the contribution this block is, as declared for
    /// this package in `gem.toml`.
    fn name(&mut self, meta: &ParseNestedMeta<'_>) -> syn::Result<()> {
        let token = meta.path.require_ident().map_err(|_| {
            syn::Error::new(
                meta.path.span(),
                "`#[contribution]` takes the contribution's id as one bare token, for example \
                 `#[contribution(runtime)]`, or the four keys `gem`, `id`, `roles`, and `caps`",
            )
        })?;
        if let Some(named) = self.token.replace(token.clone()) {
            return Err(syn::Error::new(
                token.span(),
                format!(
                    "`#[contribution]` marks one contribution, and this block is already \
                     `{named}`; a second contribution is a second type and a second block"
                ),
            ));
        }
        Ok(())
    }

    /// The two manifest forms, and the keyed one they exclude.
    fn resolve(&self) -> syn::Result<Resolved> {
        let keyed =
            self.gem.is_some() || self.id.is_some() || self.roles.is_some() || self.caps.is_some();
        match (&self.token, keyed) {
            (Some(token), true) => Err(syn::Error::new(
                token.span(),
                format!(
                    "`#[contribution({token})]` reads `{token}`'s roles and caps from `gem.toml`, \
                     so it takes no keys; spell every fact at the call site with `gem`, `id`, \
                     `roles`, and `caps`, or name the contribution and let the manifest answer"
                ),
            )),
            (token, false) => from_manifest(token.as_ref()),
            (None, true) => from_keys(self),
        }
    }
}

/// Everything codegen needs, however it was resolved. All three forms land
/// here, so there is exactly one place the descriptor and the caps declaration
/// are written.
struct Resolved {
    gem: proc_macro2::TokenStream,
    id: proc_macro2::TokenStream,
    roles: Vec<proc_macro2::TokenStream>,
    caps: Vec<Ident>,
    /// Generated products the manifest declared, in the reader's sealed
    /// spelling. The explicit form resolves none: it spells every fact at the
    /// call site and has no build script's manifest to agree with.
    products: Vec<&'static str>,
    /// The entry item, when the id was legible enough to name it.
    call: Option<Call>,
}

struct Call {
    name: Ident,
    doc: String,
    manifest: String,
}

fn expand(declaration: &Declaration, block: &ItemImpl) -> syn::Result<proc_macro2::TokenStream> {
    let trait_path = contribution_trait(block)?;
    let target = named_target(block)?;
    reject_generated_items(block)?;

    let resolved = declaration.resolve()?;

    let Resolved {
        gem,
        id,
        roles,
        caps,
        products,
        call,
    } = &resolved;
    let caps_ident = format_ident!("{target}Caps");
    let attrs = &block.attrs;
    let self_ty = &block.self_ty;
    let items = body(block, products)?;

    let entry = call.as_ref().map(|call| {
        let Call {
            name,
            doc,
            manifest,
        } = call;
        quote! {
            const _: &[u8] = ::core::include_bytes!(#manifest);

            #[doc = #doc]
            #[must_use]
            pub fn #name() -> impl ::az_gem_contract::Contribution {
                #self_ty
            }
        }
    });

    Ok(quote! {
        ::az_gem_contract::declare_caps!(pub #caps_ident: #(#caps),*);

        #(#attrs)*
        impl #trait_path for #self_ty {
            type Caps = #caps_ident;

            fn descriptor(&self) -> ::az_gem_contract::ContributionDescriptor {
                ::az_gem_contract::ContributionDescriptor {
                    gem: #gem,
                    contribution: #id,
                    roles: &[#(#roles),*],
                }
            }

            #(#items)*
        }

        #entry
    })
}

/// The author's items with every declared product's registration prepended to
/// the `register` body.
///
/// A generated product is compiled in by a build script and registered by the
/// expansion, so the manifest declaration is the whole of the author's part:
/// the call cannot be forgotten, mis-spelled, or written twice. It goes first,
/// ahead of anything the body does, because the projection is what the build
/// already decided and the body may narrow, branch, or return around it.
fn body(block: &ItemImpl, products: &[&'static str]) -> syn::Result<Vec<ImplItem>> {
    if products.is_empty() {
        return Ok(block.items.clone());
    }

    let mut items = block.items.clone();
    let register = items
        .iter_mut()
        .find_map(|item| match item {
            ImplItem::Fn(function) if function.sig.ident == "register" => Some(function),
            _ => None,
        })
        .ok_or_else(|| {
            syn::Error::new(
                block.span(),
                format!(
                    "this contribution declares the generated product{} {}, and the projection is \
                     registered from its `register` body, which this block does not have",
                    if products.len() == 1 { "" } else { "s" },
                    manifest::list(products),
                ),
            )
        })?;

    let ctx = context(&register.sig)?;
    let mut composed = products
        .iter()
        .map(|product| {
            let tokens = projection(product, &ctx).ok_or_else(|| {
                syn::Error::new(
                    register.sig.ident.span(),
                    format!("`{product}` is a declared product with no known projection"),
                )
            })?;
            syn::parse2::<Stmt>(tokens)
        })
        .collect::<syn::Result<Vec<Stmt>>>()?;
    composed.append(&mut register.block.stmts);
    register.block.stmts = composed;
    Ok(items)
}

/// The registration one sealed product's projection needs.
///
/// `None` is what a product with no projection here looks like; the unit test
/// beside this is what keeps that unreachable as the sealed set grows.
fn projection(product: &str, ctx: &Ident) -> Option<proc_macro2::TokenStream> {
    match product {
        // `az-graph-builder` writes `azoth_generated_graphs.rs` into the
        // declaring crate's `OUT_DIR`, and refuses to write it at all unless
        // the manifest declares `graphs` — so the two halves of the
        // declaration meet here, and a crate that compiled graphs it never
        // declared fails in the build script rather than shipping them
        // unregistered.
        "graphs" => Some(quote! {{
            // Machine-generated build-script output: written into OUT_DIR and
            // rewritten on every build, so it is not in git and has no
            // per-site fix. Scope the suppression to the generated module, the
            // same way `az_proto_core::generated_schema!` does for capnpc
            // output.
            #[allow(clippy::all, clippy::pedantic, clippy::nursery)]
            mod graphs {
                include!(concat!(env!("OUT_DIR"), "/azoth_generated_graphs.rs"));
            }
            graphs::register(#ctx);
        }}),
        _ => None,
    }
}

/// The name the author gave the context, so the injected call passes the
/// binding this body actually has.
fn context(signature: &Signature) -> syn::Result<Ident> {
    const SHAPE: &str = "`register` takes the typed context as its second parameter, for example `fn \
         register(&self, ctx: &mut GemContext<'_, Self::Caps>)`";

    let Some(FnArg::Typed(typed)) = signature.inputs.iter().nth(1) else {
        return Err(syn::Error::new(signature.span(), SHAPE));
    };
    let Pat::Ident(bound) = &*typed.pat else {
        return Err(syn::Error::new(typed.pat.span(), SHAPE));
    };
    Ok(bound.ident.clone())
}

/// The manifest forms: read the gem's manifest and mint the identity, with the
/// bare token — when there is one — narrowing the package's declarations to the
/// contribution this block is.
fn from_manifest(token: Option<&Ident>) -> syn::Result<Resolved> {
    let package = cargo("CARGO_PKG_NAME")?;
    let root = cargo("CARGO_MANIFEST_DIR")?;
    let named = token.map(ToString::to_string);
    let entry = manifest::read(&package, std::path::Path::new(&root), named.as_deref()).map_err(
        |message| syn::Error::new(token.map_or_else(Span::call_site, Ident::span), message),
    )?;

    let gem = entry.gem;
    let id = entry.id;
    let doc = format!(
        "Entry item for gem `{gem}`, contribution `{id}`.\n\nGenerated by `#[contribution]` from \
         the gem manifest; generated target glue constructs the contribution by calling it."
    );
    Ok(Resolved {
        gem: quote!(::az_gem_contract::GemId::new(#gem)),
        id: quote!(::az_gem_contract::ContributionId::new(#id)),
        roles: entry
            .roles
            .into_iter()
            .map(|role| quote!(::az_gem_contract::GemTargetRole::#role))
            .collect(),
        caps: entry.caps,
        products: entry.products,
        call: Some(Call {
            name: entry.call,
            doc,
            manifest: entry.path,
        }),
    })
}

/// The explicit form: every key is required, and none may be inferred from the
/// absence of the others.
fn from_keys(declaration: &Declaration) -> syn::Result<Resolved> {
    let gem = required(
        declaration.gem.as_ref(),
        "`#[contribution]` requires `gem = <GemId const>` from the gem's generated ids crate, or a manifest form — no arguments, or the contribution's id as a bare token — to take identity from `gem.toml`",
    )?;
    let id = required(
        declaration.id.as_ref(),
        "`#[contribution]` requires `id = <ContributionId const>` from the gem's generated ids crate, or a manifest form — no arguments, or the contribution's id as a bare token — to take identity from `gem.toml`",
    )?;
    let roles = required(
        declaration.roles.as_ref(),
        "`#[contribution]` requires `roles = [...]` naming the target roles this contribution composes into",
    )?;
    let caps = required(
        declaration.caps.as_ref(),
        "`#[contribution]` requires `caps = [...]`; a contribution with no host-capability floor declares `caps = []`",
    )?;

    Ok(Resolved {
        gem: quote!(#gem),
        id: quote!(#id),
        roles: roles.clone(),
        caps: caps.clone(),
        products: Vec::new(),
        call: None,
    })
}

/// Cargo's own facts about the crate being compiled, which is what makes the
/// manifest reachable without the author naming a path.
fn cargo(key: &str) -> syn::Result<String> {
    std::env::var(key).map_err(|_| {
        syn::Error::new(
            Span::call_site(),
            format!(
                "`#[contribution]` resolves identity from the gem manifest and needs Cargo's \
                 `{key}` to find it; declare identity explicitly with `gem`, `id`, `roles`, and \
                 `caps` when expanding outside Cargo"
            ),
        )
    })
}

/// The attribute applies to a plain `impl Contribution for T` block.
fn contribution_trait(block: &ItemImpl) -> syn::Result<&Path> {
    const SHAPE: &str = "`#[contribution]` applies to an `impl Contribution for T` block";

    let Some((None, path, _)) = &block.trait_ else {
        return Err(syn::Error::new(block.span(), SHAPE));
    };
    if path
        .segments
        .last()
        .is_none_or(|segment| segment.ident != "Contribution")
    {
        return Err(syn::Error::new(path.span(), SHAPE));
    }
    Ok(path)
}

/// The caps declaration is named after the entry item, so the contribution
/// type must be concrete and named.
fn named_target(block: &ItemImpl) -> syn::Result<&Ident> {
    const NAMED: &str = "`#[contribution]` requires a named contribution type";

    if !block.generics.params.is_empty() {
        return Err(syn::Error::new(
            block.generics.span(),
            "`#[contribution]` requires a concrete entry item; generic contributions are unsupported",
        ));
    }
    let Type::Path(path) = &*block.self_ty else {
        return Err(syn::Error::new(block.self_ty.span(), NAMED));
    };
    path.path
        .segments
        .last()
        .map(|segment| &segment.ident)
        .ok_or_else(|| syn::Error::new(block.self_ty.span(), NAMED))
}

fn reject_generated_items(block: &ItemImpl) -> syn::Result<()> {
    for item in &block.items {
        match item {
            ImplItem::Type(generated) if generated.ident == "Caps" => {
                return Err(syn::Error::new(
                    generated.ident.span(),
                    "`type Caps` is generated by `#[contribution]`; declare the floor with `caps = [...]` or in `gem.toml`",
                ));
            }
            ImplItem::Fn(generated) if generated.sig.ident == "descriptor" => {
                return Err(syn::Error::new(
                    generated.sig.ident.span(),
                    "`descriptor` is generated by `#[contribution]`; declare identity with `gem`, `id`, and `roles`, or in `gem.toml`",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Ids travel as consts from the gem's generated ids crate; a literal here
/// would re-mint identity outside the manifest.
fn id_const(meta: &ParseNestedMeta<'_>) -> syn::Result<Expr> {
    let key = meta.path.require_ident()?;
    let value: Expr = meta.value()?.parse()?;
    if let Expr::Lit(literal) = &value
        && matches!(literal.lit, Lit::Str(_))
    {
        return Err(syn::Error::new(
            value.span(),
            format!(
                "`{key}` takes an id const from the gem's generated ids crate (`gem_ids::GEM`), never a string literal; drop every argument to take identity from `gem.toml` instead"
            ),
        ));
    }
    Ok(value)
}

fn path_array(meta: &ParseNestedMeta<'_>, expected: &str) -> syn::Result<Vec<Path>> {
    let array: ExprArray = meta.value()?.parse()?;
    array
        .elems
        .into_iter()
        .map(|element| match element {
            Expr::Path(path) if path.qself.is_none() => Ok(path.path),
            other => Err(syn::Error::new(other.span(), expected)),
        })
        .collect()
}

/// Bare variant names resolve against the contract's role enum; a qualified
/// path is passed through as written.
fn role_tokens(paths: Vec<Path>) -> Vec<proc_macro2::TokenStream> {
    paths
        .into_iter()
        .map(|path| {
            path.get_ident().map_or_else(
                || quote!(#path),
                |variant| quote!(::az_gem_contract::GemTargetRole::#variant),
            )
        })
        .collect()
}

fn cap_idents(paths: Vec<Path>) -> syn::Result<Vec<Ident>> {
    paths
        .into_iter()
        .map(|path| {
            path.get_ident()
                .cloned()
                .ok_or_else(|| syn::Error::new(path.span(), CAPS_SHAPE))
        })
        .collect()
}

fn required<'a, T>(value: Option<&'a T>, message: &str) -> syn::Result<&'a T> {
    value.ok_or_else(|| syn::Error::new(Span::call_site(), message))
}

/// The key names itself, so a call site cannot drift from the key it parsed.
fn replace_once<T>(
    destination: &mut Option<T>,
    value: T,
    meta: &ParseNestedMeta<'_>,
) -> syn::Result<()> {
    let key = meta.path.require_ident()?;
    if destination.replace(value).is_some() {
        return Err(syn::Error::new(
            key.span(),
            format!("duplicate `contribution` key `{key}`"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{body, projection};
    use quote::{ToTokens, quote};
    use syn::ItemImpl;

    fn block() -> ItemImpl {
        syn::parse2(quote! {
            impl Contribution for Runtime {
                fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
                    ctx.registrar::<Component>().register(Component("Mount"));
                }
            }
        })
        .expect("the fixture block parses")
    }

    fn rendered(products: &[&'static str]) -> String {
        body(&block(), products)
            .expect("the fixture block has a register body")
            .iter()
            .map(|item| item.to_token_stream().to_string())
            .collect()
    }

    /// Every name the manifest reader admits has a projection here.
    ///
    /// The reader hands back the sealed set's own `&'static str`, so this is
    /// the only crossing the two tables have: a product the reader learns and
    /// this match does not fails here rather than expanding into silence.
    #[test]
    fn every_sealed_product_has_a_projection() {
        let ctx = syn::Ident::new("ctx", proc_macro2::Span::call_site());
        for product in crate::manifest::PRODUCTS {
            assert!(
                projection(product, &ctx).is_some(),
                "`{product}` is declarable and has no projection"
            );
        }
    }

    /// The projection is registered before anything the author wrote, and it
    /// is registered by `include!`ing what the build script emitted — so a
    /// crate that declared a product its build never wrote fails to compile,
    /// naming the file rustc could not read, instead of running without it.
    #[test]
    fn a_declared_product_is_the_first_statement_of_register() {
        let rendered = rendered(&["graphs"]);
        let include = rendered
            .find("include !")
            .expect("the projection is included");
        let authored = rendered
            .find("registrar")
            .expect("the author's own body survives");

        assert!(rendered.contains("azoth_generated_graphs.rs"), "{rendered}");
        assert!(rendered.contains("env ! (\"OUT_DIR\")"), "{rendered}");
        assert!(rendered.contains("graphs :: register (ctx)"), "{rendered}");
        assert!(include < authored, "{rendered}");
    }

    #[test]
    fn no_declared_product_leaves_the_body_alone() {
        let rendered = rendered(&[]);
        assert!(!rendered.contains("include !"), "{rendered}");
        assert!(rendered.contains("registrar"), "{rendered}");
    }

    /// The injected call passes the binding the body actually has, so a body
    /// that named its context something else still compiles.
    #[test]
    fn the_projection_is_passed_the_contexts_own_name() {
        let block: ItemImpl = syn::parse2(quote! {
            impl Contribution for Runtime {
                fn register(&self, context: &mut GemContext<'_, Self::Caps>) {}
            }
        })
        .expect("the fixture block parses");
        let rendered: String = body(&block, &["graphs"])
            .expect("the fixture block has a register body")
            .iter()
            .map(|item| item.to_token_stream().to_string())
            .collect();
        assert!(
            rendered.contains("graphs :: register (context)"),
            "{rendered}"
        );
    }

    /// Declaring a product on a block with nothing to register it from is the
    /// author's mistake, and the message names the products it could not
    /// place.
    #[test]
    fn a_declared_product_without_a_register_body_is_refused() {
        let block: ItemImpl =
            syn::parse2(quote! { impl Contribution for Runtime {} }).expect("the block parses");
        let message = body(&block, &["graphs"])
            .map(|_| ())
            .expect_err("there is no register body")
            .to_string();
        assert!(message.contains("`graphs`"), "{message}");
        assert!(message.contains("`register` body"), "{message}");
    }
}
