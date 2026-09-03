use crate::error::EosError;
use bevy::prelude::Resource;
use std::env;

pub const EOS_PRODUCT_ID_ENV: &str = "EOS_PRODUCT_ID";
pub const EOS_SANDBOX_ID_ENV: &str = "EOS_SANDBOX_ID";
pub const EOS_DEPLOYMENT_ID_ENV: &str = "EOS_DEPLOYMENT_ID";
pub const EOS_CLIENT_ID_ENV: &str = "EOS_CLIENT_ID";
pub const EOS_CLIENT_SECRET_ENV: &str = "EOS_CLIENT_SECRET";

#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct EosSettings {
    pub product_id: String,
    pub sandbox_id: String,
    pub deployment_id: String,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}

impl EosSettings {
    #[must_use]
    pub fn new(
        product_id: impl Into<String>,
        sandbox_id: impl Into<String>,
        deployment_id: impl Into<String>,
    ) -> Self {
        Self {
            product_id: product_id.into(),
            sandbox_id: sandbox_id.into(),
            deployment_id: deployment_id.into(),
            client_id: None,
            client_secret: None,
        }
    }

    #[must_use]
    pub fn with_client_credentials(
        mut self,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> Self {
        self.client_id = Some(client_id.into());
        self.client_secret = Some(client_secret.into());
        self
    }

    /// Reads the EOS product, sandbox, deployment, and optional client
    /// credentials out of the process environment.
    ///
    /// # Errors
    ///
    /// Returns [`EosError::InvalidConfiguration`] naming the first of
    /// `EOS_PRODUCT_ID`, `EOS_SANDBOX_ID`, or `EOS_DEPLOYMENT_ID` that is unset
    /// or empty. The client id and secret are optional and never fail.
    pub fn from_env() -> Result<Self, EosError> {
        let settings = Self::new(
            required_env(EOS_PRODUCT_ID_ENV)?,
            required_env(EOS_SANDBOX_ID_ENV)?,
            required_env(EOS_DEPLOYMENT_ID_ENV)?,
        );

        Ok(
            match (
                optional_env(EOS_CLIENT_ID_ENV),
                optional_env(EOS_CLIENT_SECRET_ENV),
            ) {
                (Some(client_id), Some(client_secret)) => {
                    settings.with_client_credentials(client_id, client_secret)
                }
                _ => settings,
            },
        )
    }
}

fn required_env(name: &str) -> Result<String, EosError> {
    optional_env(name).ok_or_else(|| {
        EosError::InvalidConfiguration(format!(
            "{name} must be supplied by the project settings resource or environment"
        ))
    })
}

fn optional_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
