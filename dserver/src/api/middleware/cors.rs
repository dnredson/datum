use std::str::FromStr;

use http::{
    header::{HeaderName, HeaderValue},
    Method,
};
use tower_http::cors::{Any, CorsLayer};

use crate::config::AppConfig;

pub fn build_cors_layer(config: &AppConfig) -> Option<CorsLayer> {
    let cors = config.cors.as_ref()?;

    if !cors.enabled.unwrap_or(false) {
        return None;
    }

    let mut layer = CorsLayer::new();

    if cors.allow_any_origin.unwrap_or(false) {
        layer = layer.allow_origin(Any);
    } else if let Some(origins) = &cors.allowed_origins {
        let values: Vec<HeaderValue> = origins
            .iter()
            .filter_map(|o| HeaderValue::from_str(o).ok())
            .collect();

        if !values.is_empty() {
            layer = layer.allow_origin(values);
        }
    }

    if let Some(methods) = &cors.allowed_methods {
        let parsed: Vec<Method> = methods
            .iter()
            .filter_map(|m| Method::from_str(m).ok())
            .collect();

        if !parsed.is_empty() {
            layer = layer.allow_methods(parsed);
        }
    }

    if let Some(headers) = &cors.allowed_headers {
        let parsed: Vec<HeaderName> = headers
            .iter()
            .filter_map(|h| HeaderName::from_str(h).ok())
            .collect();

        if !parsed.is_empty() {
            layer = layer.allow_headers(parsed);
        }
    }

    Some(layer)
}
