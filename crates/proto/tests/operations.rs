use sticktalk_proto::*;

#[test]
fn operations_round_trip_as_named_maps_without_changing_attachments() {
    let request = CreateUploadRequest {
        purpose: UploadPurpose::Message,
        channel_id: Some(Id(7)),
        filename: "large.bin".into(),
        content_type: "application/octet-stream".into(),
        byte_size: 1_u64 << 40,
        sha256: "a".repeat(64),
    };
    let bytes = rmp_serde::to_vec_named(&request).unwrap();
    assert_eq!(
        rmp_serde::from_slice::<CreateUploadRequest>(&bytes).unwrap(),
        request
    );
    assert_eq!(
        serde_json::to_value(&request).unwrap()["byte_size"],
        1_u64 << 40
    );
    let status = ResumableUpload {
        upload_id: "0".repeat(32),
        offset: 16 * 1024 * 1024,
        byte_size: 1_u64 << 40,
        chunk_size: 16 * 1024 * 1024,
        expires_at: 123,
        completed: None,
    };
    assert_eq!(
        serde_json::from_slice::<ResumableUpload>(&serde_json::to_vec(&status).unwrap()).unwrap(),
        status
    );
}

#[test]
fn signed_manifest_envelope_preserves_exact_signed_bytes() {
    let value = SignedUpdateManifest {
        manifest: "{ \"version\": \"1.0.0\", \"note\": \"é\" }\n".into(),
        signature: "base64".into(),
    };
    let json = serde_json::to_vec(&value).unwrap();
    assert_eq!(
        serde_json::from_slice::<SignedUpdateManifest>(&json).unwrap(),
        value
    );
    assert_eq!(
        rmp_serde::from_slice::<SignedUpdateManifest>(&rmp_serde::to_vec_named(&value).unwrap())
            .unwrap(),
        value
    );
}
