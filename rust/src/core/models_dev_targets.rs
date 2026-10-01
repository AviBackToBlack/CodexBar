//! models.dev pricing identities for a recorded billing route (upstream
//! 0.60.4 `ModelsDevPricingTargetResolver`).
//!
//! The recorded provider is the billing provider. A model namespace that
//! names another vendor (`openrouter` + `openai/gpt-5`) is part of the model
//! id on that provider's catalog, never a route to the vendor's own rates.

/// One provider/model identity that may price a recorded usage row.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModelsDevPricingTarget {
    pub provider_id: String,
    pub model_id: String,
}

/// Resolves the models.dev identities for a recorded provider and model.
///
/// The provider id is trimmed and lowercased (`x-ai` is models.dev `xai`); the
/// model id is trimmed. One leading segment is stripped only when it repeats
/// the provider itself (`openai` + `openai/gpt-5` is `gpt-5`). Two providers
/// also check their canonical models.dev id: `kimi-coding` adds
/// `kimi-for-coding` and `opencode-free` adds `opencode`. Empty or malformed
/// ids (an empty provider, or a model that is empty or starts or ends with
/// `/`) resolve to nothing.
pub fn models_dev_pricing_targets(
    provider_id: &str,
    model_id: &str,
) -> Vec<ModelsDevPricingTarget> {
    let provider_id = normalized_provider_id(provider_id);
    let model_id = model_id.trim();
    if provider_id.is_empty() || !is_valid_model_id(model_id) {
        return Vec::new();
    }
    let model_id = strip_self_prefix(model_id, &provider_id);
    if !is_valid_model_id(model_id) {
        return Vec::new();
    }
    let alias = match provider_id.as_str() {
        "kimi-coding" => Some("kimi-for-coding"),
        "opencode-free" => Some("opencode"),
        _ => None,
    };
    let mut targets = vec![ModelsDevPricingTarget {
        provider_id,
        model_id: model_id.to_string(),
    }];
    if let Some(alias) = alias {
        targets.push(ModelsDevPricingTarget {
            provider_id: alias.to_string(),
            model_id: model_id.to_string(),
        });
    }
    targets
}

fn normalized_provider_id(provider_id: &str) -> String {
    let normalized = provider_id.trim().to_ascii_lowercase();
    if normalized == "x-ai" {
        "xai".to_string()
    } else {
        normalized
    }
}

fn strip_self_prefix<'a>(model_id: &'a str, provider_id: &str) -> &'a str {
    match model_id.split_once('/') {
        Some((prefix, remainder))
            if !remainder.is_empty() && normalized_provider_id(prefix) == provider_id =>
        {
            remainder
        }
        _ => model_id,
    }
}

fn is_valid_model_id(model_id: &str) -> bool {
    !model_id.is_empty() && !model_id.starts_with('/') && !model_id.ends_with('/')
}

#[cfg(test)]
mod tests {
    use super::{ModelsDevPricingTarget, models_dev_pricing_targets};

    fn target(provider_id: &str, model_id: &str) -> ModelsDevPricingTarget {
        ModelsDevPricingTarget {
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
        }
    }

    #[test]
    fn normalizes_the_provider_and_trims_the_model() {
        assert_eq!(
            models_dev_pricing_targets("  x-ai  ", "  Grok-4.6  "),
            vec![target("xai", "Grok-4.6")]
        );
    }

    #[test]
    fn strips_only_one_prefix_that_repeats_the_provider() {
        assert_eq!(
            models_dev_pricing_targets("OpenAI", "openai/GPT-5"),
            vec![target("openai", "GPT-5")]
        );
        assert_eq!(
            models_dev_pricing_targets("x-ai", "x-ai/grok-4.6"),
            vec![target("xai", "grok-4.6")]
        );
        assert_eq!(
            models_dev_pricing_targets("xai", "xai/xai/grok-4.6"),
            vec![target("xai", "xai/grok-4.6")]
        );
        assert_eq!(
            models_dev_pricing_targets("google-vertex", "google-vertex/gemini-2.5-pro"),
            vec![target("google-vertex", "gemini-2.5-pro")]
        );
    }

    #[test]
    fn router_namespace_stays_part_of_the_model_id() {
        assert_eq!(
            models_dev_pricing_targets("openrouter", "openai/gpt-5"),
            vec![target("openrouter", "openai/gpt-5")]
        );
        assert_eq!(
            models_dev_pricing_targets("OpenRouter", "openrouter/openai/gpt-5"),
            vec![target("openrouter", "openai/gpt-5")]
        );
        assert_eq!(
            models_dev_pricing_targets("private-proxy", "openai/gpt-5"),
            vec![target("private-proxy", "openai/gpt-5")]
        );
    }

    #[test]
    fn canonical_aliases_follow_the_recorded_provider() {
        assert_eq!(
            models_dev_pricing_targets("kimi-coding", "kimi-coding/k3"),
            vec![target("kimi-coding", "k3"), target("kimi-for-coding", "k3")]
        );
        assert_eq!(
            models_dev_pricing_targets("opencode-free", "opencode-free/opencode"),
            vec![
                target("opencode-free", "opencode"),
                target("opencode", "opencode")
            ]
        );
        assert_eq!(
            models_dev_pricing_targets("kimi-coding", "k3"),
            vec![target("kimi-coding", "k3"), target("kimi-for-coding", "k3")]
        );
    }

    #[test]
    fn malformed_identities_resolve_to_nothing() {
        for (provider_id, model_id) in [
            ("", "gpt-5"),
            ("  ", "gpt-5"),
            ("openai", ""),
            ("openai", " / "),
            ("openai", "/gpt-5"),
            ("openai", "gpt-5/"),
            ("openai", "openai/gpt-5/"),
        ] {
            assert!(
                models_dev_pricing_targets(provider_id, model_id).is_empty(),
                "{provider_id:?} {model_id:?}"
            );
        }
    }
}
