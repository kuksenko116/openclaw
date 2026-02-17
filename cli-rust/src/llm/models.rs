// Model management: aliases, info registry, cost estimation, and context limits.

// ---------------------------------------------------------------------------
// Model aliases
// ---------------------------------------------------------------------------

/// Resolve a short alias to the canonical model ID.
///
/// Unknown names are returned as-is so callers can freely pass full model IDs.
pub(crate) fn resolve_model_alias(name: &str) -> &str {
    match name {
        "sonnet" | "claude-sonnet" => "claude-sonnet-4-20250514",
        "opus" | "claude-opus" => "claude-opus-4-20250514",
        "haiku" | "claude-haiku" => "claude-haiku-3-20250307",
        "gpt4o" | "gpt-4o" => "gpt-4o",
        "gpt4o-mini" | "gpt-4o-mini" => "gpt-4o-mini",
        "gpt4-turbo" | "gpt-4-turbo" => "gpt-4-turbo",
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Model info registry
// ---------------------------------------------------------------------------

/// Static metadata for a known model.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ModelInfo {
    pub id: &'static str,
    pub provider: &'static str,
    pub context_window: usize,
    pub max_output: usize,
    /// Price in USD per million input tokens.
    pub input_price_per_mtok: f64,
    /// Price in USD per million output tokens.
    pub output_price_per_mtok: f64,
    pub supports_thinking: bool,
    pub supports_images: bool,
}

/// Built-in model catalogue.
static MODELS: &[ModelInfo] = &[
    ModelInfo {
        id: "claude-sonnet-4-20250514",
        provider: "anthropic",
        context_window: 200_000,
        max_output: 16_000,
        input_price_per_mtok: 3.0,
        output_price_per_mtok: 15.0,
        supports_thinking: true,
        supports_images: true,
    },
    ModelInfo {
        id: "claude-opus-4-20250514",
        provider: "anthropic",
        context_window: 200_000,
        max_output: 32_000,
        input_price_per_mtok: 15.0,
        output_price_per_mtok: 75.0,
        supports_thinking: true,
        supports_images: true,
    },
    ModelInfo {
        id: "claude-haiku-3-20250307",
        provider: "anthropic",
        context_window: 200_000,
        max_output: 4_000,
        input_price_per_mtok: 0.25,
        output_price_per_mtok: 1.25,
        supports_thinking: false,
        supports_images: true,
    },
    ModelInfo {
        id: "gpt-4o",
        provider: "openai",
        context_window: 128_000,
        max_output: 16_000,
        input_price_per_mtok: 2.50,
        output_price_per_mtok: 10.0,
        supports_thinking: false,
        supports_images: true,
    },
    ModelInfo {
        id: "gpt-4o-mini",
        provider: "openai",
        context_window: 128_000,
        max_output: 16_000,
        input_price_per_mtok: 0.15,
        output_price_per_mtok: 0.60,
        supports_thinking: false,
        supports_images: true,
    },
    ModelInfo {
        id: "gpt-4-turbo",
        provider: "openai",
        context_window: 128_000,
        max_output: 4_000,
        input_price_per_mtok: 10.0,
        output_price_per_mtok: 30.0,
        supports_thinking: false,
        supports_images: true,
    },
];

/// Look up static metadata for a model by its canonical ID.
pub(crate) fn get_model_info(model: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|m| m.id == model)
}

// ---------------------------------------------------------------------------
// Cost estimation
// ---------------------------------------------------------------------------

/// Estimate the cost in USD for the given token counts.
///
/// Returns `None` when the model is not in the registry.
pub(crate) fn estimate_cost(model: &str, input_tokens: u64, output_tokens: u64) -> Option<f64> {
    let info = get_model_info(model)?;
    let input_cost = (input_tokens as f64 / 1_000_000.0) * info.input_price_per_mtok;
    let output_cost = (output_tokens as f64 / 1_000_000.0) * info.output_price_per_mtok;
    Some(input_cost + output_cost)
}

/// Pretty-print a cost value as a dollar string (e.g. `"$0.0234"`).
pub(crate) fn format_cost(cost: f64) -> String {
    if cost < 0.01 {
        format!("${:.4}", cost)
    } else {
        format!("${:.2}", cost)
    }
}

// ---------------------------------------------------------------------------
// Context / output limits
// ---------------------------------------------------------------------------

const DEFAULT_CONTEXT_WINDOW: usize = 128_000;
const DEFAULT_MAX_OUTPUT: usize = 4_096;

/// Return the context window size for `model`, falling back to 128 000.
pub(crate) fn context_limit(model: &str) -> usize {
    get_model_info(model)
        .map(|m| m.context_window)
        .unwrap_or(DEFAULT_CONTEXT_WINDOW)
}

/// Return the maximum output token count for `model`, falling back to 4 096.
pub(crate) fn max_output_tokens(model: &str) -> usize {
    get_model_info(model)
        .map(|m| m.max_output)
        .unwrap_or(DEFAULT_MAX_OUTPUT)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Alias resolution ---------------------------------------------------

    #[test]
    fn alias_sonnet() {
        assert_eq!(resolve_model_alias("sonnet"), "claude-sonnet-4-20250514");
        assert_eq!(
            resolve_model_alias("claude-sonnet"),
            "claude-sonnet-4-20250514"
        );
    }

    #[test]
    fn alias_opus() {
        assert_eq!(resolve_model_alias("opus"), "claude-opus-4-20250514");
        assert_eq!(resolve_model_alias("claude-opus"), "claude-opus-4-20250514");
    }

    #[test]
    fn alias_haiku() {
        assert_eq!(resolve_model_alias("haiku"), "claude-haiku-3-20250307");
        assert_eq!(
            resolve_model_alias("claude-haiku"),
            "claude-haiku-3-20250307"
        );
    }

    #[test]
    fn alias_gpt4o() {
        assert_eq!(resolve_model_alias("gpt4o"), "gpt-4o");
        assert_eq!(resolve_model_alias("gpt-4o"), "gpt-4o");
    }

    #[test]
    fn alias_gpt4o_mini() {
        assert_eq!(resolve_model_alias("gpt4o-mini"), "gpt-4o-mini");
        assert_eq!(resolve_model_alias("gpt-4o-mini"), "gpt-4o-mini");
    }

    #[test]
    fn alias_gpt4_turbo() {
        assert_eq!(resolve_model_alias("gpt4-turbo"), "gpt-4-turbo");
        assert_eq!(resolve_model_alias("gpt-4-turbo"), "gpt-4-turbo");
    }

    #[test]
    fn alias_passthrough() {
        assert_eq!(resolve_model_alias("my-custom-model"), "my-custom-model");
        assert_eq!(
            resolve_model_alias("claude-sonnet-4-20250514"),
            "claude-sonnet-4-20250514"
        );
    }

    // -- Model info lookup --------------------------------------------------

    #[test]
    fn info_known_model() {
        let info = get_model_info("claude-sonnet-4-20250514").expect("should find sonnet");
        assert_eq!(info.provider, "anthropic");
        assert_eq!(info.context_window, 200_000);
        assert_eq!(info.max_output, 16_000);
        assert!(info.supports_thinking);
        assert!(info.supports_images);
    }

    #[test]
    fn info_all_models_present() {
        let ids = [
            "claude-sonnet-4-20250514",
            "claude-opus-4-20250514",
            "claude-haiku-3-20250307",
            "gpt-4o",
            "gpt-4o-mini",
            "gpt-4-turbo",
        ];
        for id in ids {
            assert!(
                get_model_info(id).is_some(),
                "model {id} should be in registry"
            );
        }
    }

    #[test]
    fn info_unknown_model() {
        assert!(get_model_info("unknown-model-xyz").is_none());
    }

    // -- Cost estimation ----------------------------------------------------

    #[test]
    fn cost_estimation_sonnet() {
        // 1M input tokens at $3, 1M output tokens at $15 => $18
        let cost = estimate_cost("claude-sonnet-4-20250514", 1_000_000, 1_000_000);
        assert_eq!(cost, Some(18.0));
    }

    #[test]
    fn cost_estimation_small() {
        // 1000 input at $3/M, 500 output at $15/M
        let cost = estimate_cost("claude-sonnet-4-20250514", 1_000, 500).unwrap();
        let expected = 1_000.0 / 1_000_000.0 * 3.0 + 500.0 / 1_000_000.0 * 15.0;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn cost_estimation_unknown_model() {
        assert!(estimate_cost("nonexistent", 1000, 1000).is_none());
    }

    #[test]
    fn cost_format_small() {
        assert_eq!(format_cost(0.00234), "$0.0023");
    }

    #[test]
    fn cost_format_large() {
        assert_eq!(format_cost(1.23), "$1.23");
    }

    #[test]
    fn cost_format_boundary() {
        // Exactly at the boundary: < 0.01 uses 4 decimals
        assert_eq!(format_cost(0.009), "$0.0090");
        assert_eq!(format_cost(0.01), "$0.01");
    }

    // -- Context limits -----------------------------------------------------

    #[test]
    fn context_limit_known() {
        assert_eq!(context_limit("claude-opus-4-20250514"), 200_000);
        assert_eq!(context_limit("gpt-4o"), 128_000);
    }

    #[test]
    fn context_limit_fallback() {
        assert_eq!(context_limit("unknown-model"), 128_000);
    }

    #[test]
    fn max_output_known() {
        assert_eq!(max_output_tokens("claude-opus-4-20250514"), 32_000);
        assert_eq!(max_output_tokens("claude-haiku-3-20250307"), 4_000);
        assert_eq!(max_output_tokens("gpt-4o"), 16_000);
    }

    #[test]
    fn max_output_fallback() {
        assert_eq!(max_output_tokens("unknown-model"), 4_096);
    }
}
