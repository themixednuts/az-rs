use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use az_secrets::{
    BackendBuildContext, BackendCapabilities, BackendFactory, BackendFactoryFuture,
    LOCAL_BACKEND_ID, ResolvedSecret, SecretBackend, SecretBackendRegistry, SecretError,
    SecretFuture, SecretMountConfig, SecretRef, SecretRouter,
};

struct MapBackend {
    values: BTreeMap<String, Vec<u8>>,
    calls: Mutex<Vec<String>>,
}

impl MapBackend {
    fn with(values: impl IntoIterator<Item = (&'static str, &'static [u8])>) -> Self {
        Self {
            values: values
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value.to_vec()))
                .collect(),
            calls: Mutex::default(),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("call recorder lock").clone()
    }
}

impl SecretBackend for MapBackend {
    fn resolve<'a>(&'a self, secret: &'a SecretRef) -> SecretFuture<'a> {
        self.calls
            .lock()
            .expect("call recorder lock")
            .push(secret.as_str().to_owned());
        let result = self.values.get(secret.as_str()).cloned().map_or_else(
            || {
                Err(SecretError::Missing {
                    reference: secret.clone(),
                })
            },
            |value| Ok(ResolvedSecret::from_bytes(value)),
        );
        Box::pin(async move { result })
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::LOCAL_BINARY
    }
}

#[test]
fn secret_reference_is_fail_closed_and_transparent_in_config() {
    let reference =
        SecretRef::parse("secret://project/auth-signing-key").expect("valid logical secret path");

    assert_eq!(reference.mount(), "project");
    assert_eq!(reference.path(), "project/auth-signing-key");
    assert_eq!(reference.path_within_mount(), "auth-signing-key");
    assert_eq!(
        serde_json::to_string(&reference).expect("serialize reference"),
        r#""secret://project/auth-signing-key""#
    );
    assert!(SecretRef::parse("inline-material").is_err());
    assert!(SecretRef::parse("secret://").is_err());
    assert!(SecretRef::parse("secret://project/../escape").is_err());
    assert!(SecretRef::parse("secret://project\\escape").is_err());
    assert!(SecretRef::parse("secret://project/two words").is_err());
}

#[test]
fn resolved_material_is_zeroizing_redacted_and_bytes_first() {
    let secret = ResolvedSecret::from_bytes(b"binary\0value".to_vec());

    assert_eq!(format!("{secret:?}"), "Secret([REDACTED])");
    assert_eq!(secret.as_bytes(), b"binary\0value");
    assert_eq!(secret.as_utf8().expect("valid utf8"), "binary\0value");
    assert_eq!(&*secret.into_bytes(), b"binary\0value");

    assert!(ResolvedSecret::from_bytes(vec![0xff]).as_utf8().is_err());
}

#[test]
fn router_selects_exactly_one_mount_and_never_falls_back() {
    let project = Arc::new(MapBackend::with([(
        "secret://project/auth-signing-key",
        b"project-key".as_slice(),
    )]));
    let shared = Arc::new(MapBackend::with([(
        "secret://shared/api-key",
        b"shared-key".as_slice(),
    )]));
    let router = SecretRouter::new()
        .mount("project", project.clone())
        .expect("project mount")
        .mount("shared", shared.clone())
        .expect("shared mount");

    let secret = futures::executor::block_on(
        router.resolve(&SecretRef::parse("secret://shared/api-key").expect("shared reference")),
    )
    .expect("resolve exact mount");
    assert_eq!(secret.as_bytes(), b"shared-key");
    assert!(project.calls().is_empty());
    assert_eq!(shared.calls(), ["secret://shared/api-key"]);

    let missing_mount = futures::executor::block_on(
        router.resolve(&SecretRef::parse("secret://unknown/api-key").expect("unknown reference")),
    );
    assert!(matches!(
        missing_mount,
        Err(SecretError::MountNotConfigured { .. })
    ));
}

#[test]
fn default_batch_resolution_is_ordered_and_all_or_nothing() {
    let backend = MapBackend::with([
        ("secret://project/one", b"one".as_slice()),
        ("secret://project/three", b"three".as_slice()),
    ]);
    let refs = [
        SecretRef::parse("secret://project/one").expect("first reference"),
        SecretRef::parse("secret://project/missing").expect("missing reference"),
        SecretRef::parse("secret://project/three").expect("third reference"),
    ];

    let result = futures::executor::block_on(backend.resolve_many(&refs));

    assert!(matches!(result, Err(SecretError::Missing { .. })));
    assert_eq!(
        backend.calls(),
        ["secret://project/one", "secret://project/missing"]
    );
}

struct FixedFactory(Arc<dyn SecretBackend>);

impl BackendFactory for FixedFactory {
    fn build<'a>(
        &'a self,
        _mount: &'a str,
        _config: &'a SecretMountConfig,
        _context: BackendBuildContext<'a>,
    ) -> BackendFactoryFuture<'a> {
        let backend = self.0.clone();
        Box::pin(async move { Ok(backend) })
    }
}

#[test]
fn registry_builds_only_explicit_backend_ids_and_rejects_collisions() {
    let backend: Arc<dyn SecretBackend> = Arc::new(MapBackend::with([(
        "secret://shared/key",
        b"value".as_slice(),
    )]));
    let mut registry = SecretBackendRegistry::new();
    registry
        .register("test", Arc::new(FixedFactory(backend.clone())))
        .expect("first registration");
    assert!(matches!(
        registry.register("two words", Arc::new(FixedFactory(backend.clone()))),
        Err(SecretError::InvalidBackendConfiguration)
    ));
    assert!(matches!(
        registry.register("test", Arc::new(FixedFactory(backend))),
        Err(SecretError::DuplicateBackend { .. })
    ));

    let mounts = BTreeMap::from([("shared".to_owned(), SecretMountConfig::new("test"))]);
    let router = futures::executor::block_on(registry.build_router(
        &mounts,
        BackendBuildContext {
            project_name: "sample",
            project_root: std::path::Path::new("sample"),
        },
    ))
    .expect("registered backend builds");
    let value = futures::executor::block_on(
        router.resolve(&SecretRef::parse("secret://shared/key").unwrap()),
    )
    .expect("routed value");
    assert_eq!(value.as_bytes(), b"value");

    let unknown = BTreeMap::from([("other".to_owned(), SecretMountConfig::new("missing"))]);
    assert!(matches!(
        futures::executor::block_on(registry.build_router(
            &unknown,
            BackendBuildContext {
                project_name: "sample",
                project_root: std::path::Path::new("sample"),
            },
        )),
        Err(SecretError::BackendNotRegistered { .. })
    ));
}

#[test]
fn project_mount_defaults_to_local_but_an_explicit_route_wins() {
    let local: Arc<dyn SecretBackend> = Arc::new(MapBackend::with([(
        "secret://project/key",
        b"local".as_slice(),
    )]));
    let remote: Arc<dyn SecretBackend> = Arc::new(MapBackend::with([(
        "secret://project/key",
        b"remote".as_slice(),
    )]));
    let mut registry = SecretBackendRegistry::new();
    registry
        .register(LOCAL_BACKEND_ID, Arc::new(FixedFactory(local)))
        .unwrap();
    registry
        .register("remote", Arc::new(FixedFactory(remote)))
        .unwrap();
    let context = BackendBuildContext {
        project_name: "sample",
        project_root: std::path::Path::new("sample"),
    };
    let reference = SecretRef::parse("secret://project/key").unwrap();

    let default_router =
        futures::executor::block_on(registry.build_project_router(&BTreeMap::new(), context))
            .unwrap();
    assert_eq!(
        futures::executor::block_on(default_router.resolve(&reference))
            .unwrap()
            .as_bytes(),
        b"local"
    );

    let explicit = BTreeMap::from([("project".to_owned(), SecretMountConfig::new("remote"))]);
    let explicit_router =
        futures::executor::block_on(registry.build_project_router(&explicit, context)).unwrap();
    assert_eq!(
        futures::executor::block_on(explicit_router.resolve(&reference))
            .unwrap()
            .as_bytes(),
        b"remote"
    );
}
