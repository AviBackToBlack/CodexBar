//! Opt-in extra provider breakdowns (LiteLLM model activity, Claude workspace
//! spend). Off by default because each one costs an extra request or widens
//! the Admin API query.

use super::Settings;
use crate::core::ProviderId;

/// Providers that expose exactly one optional detail breakdown.
pub fn provider_has_optional_details(id: ProviderId) -> bool {
    matches!(id, ProviderId::LiteLLM | ProviderId::Claude)
}

impl Settings {
    /// Whether the user opted in to `id`'s optional breakdown. Always false
    /// for providers without one.
    pub fn optional_details_enabled(&self, id: ProviderId) -> bool {
        provider_has_optional_details(id)
            && self
                .provider_configs
                .get(&id)
                .is_some_and(|config| config.optional_details_enabled)
    }

    pub fn set_optional_details_enabled(&mut self, id: ProviderId, value: bool) {
        if provider_has_optional_details(id) {
            self.provider_config_mut(id).optional_details_enabled = value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ProviderConfig;

    #[test]
    fn optional_details_default_off_and_scoped_to_supporting_providers() {
        let mut settings = Settings::default();
        assert!(!settings.optional_details_enabled(ProviderId::LiteLLM));
        assert!(!settings.optional_details_enabled(ProviderId::Claude));

        settings.set_optional_details_enabled(ProviderId::LiteLLM, true);
        assert!(settings.optional_details_enabled(ProviderId::LiteLLM));
        assert!(!settings.optional_details_enabled(ProviderId::Claude));

        settings.set_optional_details_enabled(ProviderId::Codex, true);
        assert!(!settings.optional_details_enabled(ProviderId::Codex));
    }

    #[test]
    fn optional_details_round_trips_and_stays_out_of_default_json() {
        let mut config = ProviderConfig::default();
        let encoded = serde_json::to_value(&config).unwrap();
        assert!(encoded.get("optional_details_enabled").is_none());

        config.optional_details_enabled = true;
        let encoded = serde_json::to_string(&config).unwrap();
        let decoded: ProviderConfig = serde_json::from_str(&encoded).unwrap();
        assert!(decoded.optional_details_enabled);
    }
}
