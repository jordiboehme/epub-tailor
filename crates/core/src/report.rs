use serde::Serialize;

/// Summary of what a conversion did, for either human-readable or JSON output.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ConvertReport {
    pub transformations: Vec<Transformation>,
    pub warnings: Vec<Warning>,
    pub stats: ConvertStats,
}

/// A single change the converter made to the input (e.g. an image transcode, a
/// table linearization, a chapter split).
#[derive(Debug, Clone, Serialize, Default)]
pub struct Transformation {
    pub kind: String,
    pub detail: String,
    pub file: Option<String>,
}

/// A non-fatal issue encountered during conversion.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Warning {
    pub message: String,
    pub file: Option<String>,
}

/// Aggregate counters describing a conversion run.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ConvertStats {
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub images_processed: u32,
    /// Spine chapter count after any oversize-chapter splitting.
    pub chapters: u32,
    /// How many original chapters were split into parts (not the number of
    /// parts produced).
    pub chapters_split: u32,
    /// Total warnings recorded during the conversion.
    pub warnings: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_report_serializes_to_json() {
        let report = ConvertReport::default();
        let json = serde_json::to_string(&report).expect("report should serialize");
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value.get("transformations").is_some());
        assert!(value.get("warnings").is_some());
        assert!(value.get("stats").is_some());
    }

    #[test]
    fn populated_report_serializes_expected_fields() {
        let report = ConvertReport {
            transformations: vec![Transformation {
                kind: "image".to_string(),
                detail: "transcoded to grayscale JPEG".to_string(),
                file: Some("images/cover.jpg".to_string()),
            }],
            warnings: vec![Warning {
                message: "embedded font stripped".to_string(),
                file: None,
            }],
            stats: ConvertStats {
                bytes_in: 1024,
                bytes_out: 512,
                images_processed: 1,
                chapters: 3,
                chapters_split: 1,
                warnings: 1,
            },
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("transcoded to grayscale JPEG"));
        assert!(json.contains("embedded font stripped"));
        assert!(json.contains("\"chapters\":3"));
        assert!(json.contains("\"chapters_split\":1"));
        assert!(json.contains("\"warnings\":1"));
    }
}
