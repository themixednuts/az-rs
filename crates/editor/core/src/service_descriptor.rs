use az_proto_core::{ProtocolVersion, ServiceDescriptor};
use uuid::Uuid;

use crate::error::{EditorError, EditorResult};

pub fn validate_descriptor_capability_templates(
    descriptor: &ServiceDescriptor,
    service_label: &'static str,
) -> EditorResult<()> {
    descriptor
        .require_protocol_version(ProtocolVersion::CURRENT)
        .map_err(|error| {
            EditorError::ServiceDiscovery(format!(
                "{service_label} unavailable until restarted: {error}"
            ))
        })?;
    descriptor
        .validate_brokered_capability_templates()
        .map_err(|error| {
            EditorError::ServiceDiscovery(format!(
                "{service_label} descriptor capability templates are invalid: {error}"
            ))
        })
}

pub fn validate_session_descriptor_capability_templates(
    descriptor: &ServiceDescriptor,
    service_label: &'static str,
    session_id: Uuid,
) -> EditorResult<()> {
    if session_id.is_nil() {
        return Err(EditorError::ServiceDiscovery(format!(
            "{service_label} descriptor session id must not be nil"
        )));
    }
    validate_descriptor_capability_templates(descriptor, service_label)?;
    for (index, capability) in descriptor.capabilities.iter().enumerate() {
        if capability.session != Some(session_id) {
            let actual = capability
                .session
                .map_or_else(|| "<unscoped>".to_string(), |session| session.to_string());
            return Err(EditorError::ServiceDiscovery(format!(
                "{service_label} descriptor capability template {index} targets session `{actual}`, expected `{session_id}`"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use az_proto_core::{Capability, Endpoint, ServiceId, ServiceRole};

    use super::*;

    fn descriptor_with_capability(capability: Capability) -> ServiceDescriptor {
        ServiceDescriptor::new(
            ServiceId::new("azoth", "project-host"),
            ServiceRole::ProjectHost,
            Endpoint::new(az_proto_core::EndpointKind::Tcp, "127.0.0.1:1"),
        )
        .with_capability(capability)
    }

    #[test]
    fn descriptor_capability_templates_are_required() {
        let descriptor = ServiceDescriptor::new(
            ServiceId::new("azoth", "project-host"),
            ServiceRole::ProjectHost,
            Endpoint::new(az_proto_core::EndpointKind::Tcp, "127.0.0.1:1"),
        );

        let error = validate_descriptor_capability_templates(&descriptor, "project-host")
            .expect_err("empty descriptor capabilities should be rejected");

        assert!(matches!(
            error,
            EditorError::ServiceDiscovery(message)
                if message.contains("brokered capability templates")
        ));
    }

    #[test]
    fn outdated_descriptor_is_rejected_before_transport_connect() {
        let brokered = Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_audience("project-host")
            .with_permissions(["project.schema"])
            .with_token_hash([1, 2, 3]);
        let mut descriptor = descriptor_with_capability(brokered);
        descriptor.protocol = ProtocolVersion {
            major: 0,
            minor: 1,
            patch: 0,
        };

        let error = validate_descriptor_capability_templates(&descriptor, "project-host")
            .expect_err("outdated descriptor should be rejected");

        assert!(matches!(
            error,
            EditorError::ServiceDiscovery(message)
                if message.contains("project-host unavailable until restarted")
                    && message.contains("0.1.0")
                    && message.contains("0.3.0")
        ));
    }

    #[test]
    fn descriptor_capability_templates_must_be_brokered() {
        let capability = Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_audience("project-host")
            .with_permissions(["project.schema"]);
        let descriptor = descriptor_with_capability(capability);

        let error = validate_descriptor_capability_templates(&descriptor, "project-host")
            .expect_err("tokenless capability should be rejected");

        assert!(matches!(
            error,
            EditorError::ServiceDiscovery(message)
                if message.contains("brokered token hash")
        ));
    }

    #[test]
    fn descriptor_capability_templates_must_not_be_expired() {
        let capability = Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_audience("project-host")
            .with_permissions(["project.schema"])
            .with_token_hash([1, 2, 3])
            .with_expires_unix_ms(1);
        let descriptor = descriptor_with_capability(capability);

        let error = validate_descriptor_capability_templates(&descriptor, "project-host")
            .expect_err("expired capability should be rejected");

        assert!(matches!(
            error,
            EditorError::ServiceDiscovery(message)
                if message.contains("lifetime is invalid")
        ));
    }

    #[test]
    fn descriptor_capability_templates_reject_mixed_unbrokered_entries() {
        let brokered = Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_audience("project-host")
            .with_permissions(["project.schema"])
            .with_token_hash([1, 2, 3]);
        let tokenless = Capability::new(ServiceId::new("azoth", "daemon"), ServiceRole::Daemon)
            .with_audience("project-host")
            .with_permissions(["project.schema"]);
        let descriptor = descriptor_with_capability(brokered).with_capability(tokenless);

        let error = validate_descriptor_capability_templates(&descriptor, "project-host")
            .expect_err("mixed brokered and unbrokered capabilities should be rejected");

        assert!(matches!(
            error,
            EditorError::ServiceDiscovery(message)
                if message.contains("capability template 1 is not brokered")
        ));
    }

    #[test]
    fn session_descriptor_capability_templates_must_match_attached_session() {
        let session = uuid::Uuid::from_bytes([0x42; 16]);
        let capability = Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_audience("project-host")
            .with_permissions(["project.schema"])
            .with_token_hash([1, 2, 3])
            .with_session(session);
        let descriptor = descriptor_with_capability(capability);

        validate_session_descriptor_capability_templates(&descriptor, "project-host", session)
            .unwrap();

        let wrong_session = uuid::Uuid::from_bytes([0x43; 16]);
        let error = validate_session_descriptor_capability_templates(
            &descriptor,
            "project-host",
            wrong_session,
        )
        .expect_err("wrong-session descriptor capability should be rejected");

        assert!(matches!(
            error,
            EditorError::ServiceDiscovery(message)
                if message.contains("expected `43434343-4343-4343-4343-434343434343`")
        ));
    }

    #[test]
    fn session_descriptor_capability_templates_reject_nil_attached_session() {
        let session = uuid::Uuid::from_bytes([0x44; 16]);
        let capability = Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_audience("project-host")
            .with_permissions(["project.schema"])
            .with_token_hash([1, 2, 3])
            .with_session(session);
        let descriptor = descriptor_with_capability(capability);

        let error = validate_session_descriptor_capability_templates(
            &descriptor,
            "project-host",
            uuid::Uuid::nil(),
        )
        .expect_err("nil attached session id should be rejected");

        assert!(matches!(
            error,
            EditorError::ServiceDiscovery(message)
                if message.contains("session id must not be nil")
        ));
    }
}
