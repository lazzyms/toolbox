use std::path::PathBuf;

/// The watermark options shared by the image watermark and the PDF watermark.
/// An omitted option keeps the engine's existing behavior.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatermarkOptions {
    #[serde(default)] pub text: Option<String>,
    #[serde(default)] pub logo_path: Option<PathBuf>,
    /// Height of the mark as a fraction of the target's shorter side.
    #[serde(default)] pub size: Option<f32>,
    pub opacity: u8,
    #[serde(default)] pub color: Option<String>,
    #[serde(default)] pub position: Option<WatermarkPosition>,
    #[serde(default)] pub rotation: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(untagged)]
pub enum WatermarkPosition {
    Placement(WatermarkPlacement),
    /// Distance in target pixels (images) or points (PDF pages) from the top-left corner.
    Offset { x: u32, y: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WatermarkPlacement { Center, TopLeft, TopRight, BottomLeft, BottomRight, Tile }

impl WatermarkOptions {
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref().filter(|text| !text.trim().is_empty())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.opacity > 100 { return Err("Watermark opacity must be between 0 and 100.".to_string()); }
        if self.text().is_none() && self.logo_path.is_none() { return Err("Provide watermark text or a logo image.".to_string()); }
        if self.size.is_some_and(|size| !(size > 0.0 && size <= 1.0)) { return Err("Watermark size must be more than 0 and at most 1.".to_string()); }
        if let Some(color) = &self.color {
            if !(color.len() == 7 && color.starts_with('#') && color[1..].chars().all(|digit| digit.is_ascii_hexdigit())) {
                return Err("Watermark colour must be a hex colour such as #FFFFFF.".to_string());
            }
            if self.text().is_none() { return Err("Watermark colour applies to text only.".to_string()); }
        }
        if !(-180.0..=180.0).contains(&self.rotation) { return Err("Watermark rotation must be between -180 and 180 degrees.".to_string()); }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::kit::images::tools::{ImageEditPlan, WatermarkRequest};
    use crate::kit::pdf::remaining::PdfWatermarkRequest;
    use serde_json::{json, Value};

    fn with(base: Value, options: &Value) -> Value {
        let mut value = base;
        value.as_object_mut().unwrap().extend(options.as_object().unwrap().clone());
        value
    }

    fn image_request(options: &Value) -> Result<(), String> {
        let request: WatermarkRequest = serde_json::from_value(with(json!({ "paths": [], "outputLocation": "alongsideInput" }), options)).map_err(|error| error.to_string())?;
        request.options.validate()
    }

    fn image_plan(options: &Value) -> Result<(), String> {
        let plan: ImageEditPlan = serde_json::from_value(json!({ "edits": [with(json!({ "kind": "watermark" }), options)], "outputLocation": "alongsideInput" })).map_err(|error| error.to_string())?;
        plan.validate()
    }

    fn pdf_request(options: &Value) -> Result<(), String> {
        let request: PdfWatermarkRequest = serde_json::from_value(with(json!({ "paths": [], "outputLocation": "alongsideInput" }), options)).map_err(|error| error.to_string())?;
        request.options.validate()
    }

    #[test]
    fn image_and_pdf_watermarks_accept_and_reject_the_same_options() {
        let valid = [
            json!({ "text": "Toolbox", "opacity": 20 }),
            json!({ "text": "Toolbox", "opacity": 0, "position": { "x": 0, "y": 0 } }),
            json!({ "text": "Toolbox", "opacity": 100, "position": { "x": 16, "y": 16 } }),
            json!({ "logoPath": "/local/logo.png", "opacity": 70 }),
            json!({ "text": "Toolbox", "logoPath": "/local/logo.png", "opacity": 70 }),
            json!({ "text": "DRAFT", "opacity": 40, "position": "center" }),
            json!({ "text": "DRAFT", "opacity": 40, "position": "top-left" }),
            json!({ "text": "DRAFT", "opacity": 40, "position": "top-right" }),
            json!({ "text": "DRAFT", "opacity": 40, "position": "bottom-left" }),
            json!({ "text": "DRAFT", "opacity": 40, "position": "bottom-right" }),
            json!({ "text": "DRAFT", "opacity": 40, "position": "tile" }),
            json!({ "logoPath": "/local/logo.png", "opacity": 40, "position": "tile", "rotation": -45, "size": 0.25 }),
            json!({ "text": "DRAFT", "opacity": 40, "size": 1 }),
            json!({ "text": "DRAFT", "opacity": 40, "size": 0.05 }),
            json!({ "text": "DRAFT", "opacity": 40, "color": "#1a2B3c" }),
            json!({ "text": "DRAFT", "opacity": 40, "rotation": 180 }),
            json!({ "text": "DRAFT", "opacity": 40, "rotation": -180 }),
            json!({ "text": "DRAFT", "opacity": 40, "rotation": 22.5 }),
        ];
        let invalid = [
            (json!({ "text": "DRAFT", "opacity": 101 }), "Watermark opacity must be between 0 and 100."),
            (json!({ "opacity": 20 }), "Provide watermark text or a logo image."),
            (json!({ "text": "   ", "opacity": 20 }), "Provide watermark text or a logo image."),
            (json!({ "text": "DRAFT", "opacity": 20, "size": 0 }), "Watermark size must be more than 0 and at most 1."),
            (json!({ "text": "DRAFT", "opacity": 20, "size": -0.5 }), "Watermark size must be more than 0 and at most 1."),
            (json!({ "text": "DRAFT", "opacity": 20, "size": 1.5 }), "Watermark size must be more than 0 and at most 1."),
            (json!({ "text": "DRAFT", "opacity": 20, "color": "red" }), "Watermark colour must be a hex colour such as #FFFFFF."),
            (json!({ "text": "DRAFT", "opacity": 20, "color": "#FFF" }), "Watermark colour must be a hex colour such as #FFFFFF."),
            (json!({ "text": "DRAFT", "opacity": 20, "color": "#GGGGGG" }), "Watermark colour must be a hex colour such as #FFFFFF."),
            (json!({ "logoPath": "/local/logo.png", "opacity": 20, "color": "#000000" }), "Watermark colour applies to text only."),
            (json!({ "text": "DRAFT", "opacity": 20, "rotation": 181 }), "Watermark rotation must be between -180 and 180 degrees."),
            (json!({ "text": "DRAFT", "opacity": 20, "rotation": -360 }), "Watermark rotation must be between -180 and 180 degrees."),
            (json!({ "text": "DRAFT", "opacity": 300 }), ""),
            (json!({ "text": "DRAFT" }), ""),
            (json!({ "text": "DRAFT", "opacity": 20, "position": "middle" }), ""),
            (json!({ "text": "DRAFT", "opacity": 20, "position": { "x": -1, "y": 0 } }), ""),
        ];

        for options in &valid {
            for (target, result) in [("image request", image_request(options)), ("image edit plan", image_plan(options)), ("pdf request", pdf_request(options))] {
                assert_eq!(result, Ok(()), "{target} rejected {options}");
            }
        }
        for (options, message) in &invalid {
            let pdf = pdf_request(options).expect_err(&format!("pdf request accepted {options}"));
            assert_eq!(image_request(options).expect_err(&format!("image request accepted {options}")), pdf, "{options}");
            if !message.is_empty() {
                assert_eq!(pdf, *message, "{options}");
                assert_eq!(image_plan(options), Err(pdf), "{options}");
            } else {
                assert!(image_plan(options).is_err(), "image edit plan accepted {options}");
            }
        }
    }
}
