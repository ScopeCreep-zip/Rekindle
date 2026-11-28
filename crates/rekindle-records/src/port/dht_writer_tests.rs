use rekindle_secrets::record_writer::RecordWriterSecret;

use super::WriterKey;

fn writer_from_fixture(control: &str) -> WriterKey {
    WriterKey::new(
        "VLD0:fixture-owner".to_owned(),
        RecordWriterSecret::from_text(control),
    )
}

#[test]
fn writer_key_redacts_all_fields_in_originals_and_clones() {
    let control = "synthetic-writer-credential";
    let writer = writer_from_fixture(control);
    for item in [&writer, &writer.clone()] {
        let output = format!("{item:?}");
        assert!(
            !output.contains(control),
            "DHT writer Debug exposed its fixture credential"
        );
        assert_eq!(output, "WriterKey(<redacted>)");
    }
}
