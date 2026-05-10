use crate::{Recording, RecordingError};

/// Parses a canonical JSON recording and validates its semantic invariants.
pub fn from_json_str(input: &str) -> Result<Recording, RecordingError> {
    let recording: Recording = serde_json::from_str(input)?;
    recording.validate()?;
    Ok(recording)
}

/// Serializes a recording as pretty-printed canonical JSON.
pub fn to_json_pretty(recording: &Recording) -> Result<String, RecordingError> {
    recording.validate()?;
    Ok(serde_json::to_string_pretty(recording)?)
}

#[cfg(test)]
mod tests {
    use crate::{
        from_json_str, to_json_pretty, AbsoluteScreenPoint, DisplayMetadata, EventOffset,
        InputAction, InputEvent, KeyDescriptor, Recording, RecordingMetadata, ScanCode,
        SchemaVersion, ScreenSize,
    };

    fn sample_recording() -> Recording {
        Recording::new(
            RecordingMetadata {
                schema_version: SchemaVersion::new(1),
                title: Some(String::from("Round trip")),
                display: DisplayMetadata {
                    virtual_origin: AbsoluteScreenPoint { x: 0, y: 0 },
                    virtual_size: ScreenSize {
                        width: 1920,
                        height: 1080,
                    },
                    monitors: Vec::new(),
                },
            },
            vec![
                InputEvent {
                    sequence: 0,
                    offset: EventOffset::from_micros(0),
                    action: InputAction::KeyPressed {
                        key: KeyDescriptor {
                            scan_code: ScanCode::new(30),
                            logical_name: Some(String::from("A")),
                            extended: false,
                        },
                    },
                },
                InputEvent {
                    sequence: 1,
                    offset: EventOffset::from_micros(35_000),
                    action: InputAction::KeyReleased {
                        key: KeyDescriptor {
                            scan_code: ScanCode::new(30),
                            logical_name: Some(String::from("A")),
                            extended: false,
                        },
                    },
                },
            ],
        )
        .expect("sample recording should be valid")
    }

    #[test]
    fn canonical_json_round_trip_preserves_event_order() {
        let recording = sample_recording();
        let encoded = to_json_pretty(&recording).expect("recording should serialize");
        let decoded = from_json_str(&encoded).expect("recording should deserialize");

        assert_eq!(decoded, recording);
        assert_eq!(decoded.events()[0].sequence, 0);
        assert_eq!(decoded.events()[1].sequence, 1);
    }

    #[test]
    fn import_rejects_out_of_range_normalized_coordinates() {
        let invalid_json = r#"
        {
          "metadata": {
            "schema_version": 1,
            "display": {
              "virtual_origin": { "x": 0, "y": 0 },
              "virtual_size": { "width": 1920, "height": 1080 },
              "monitors": []
            }
          },
          "events": [
            {
              "sequence": 0,
              "offset": 0,
              "action": {
                "type": "pointer_moved",
                "position": {
                  "absolute": { "x": 400, "y": 300 },
                  "normalized": { "x": 1.25, "y": 0.5 }
                }
              }
            }
          ]
        }
        "#;

        let error = from_json_str(invalid_json).expect_err("import should reject invalid data");
        let rendered = error.to_string();

        assert!(rendered.contains("normalized coordinate"));
    }
}
