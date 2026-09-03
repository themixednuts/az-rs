use az_secret_value::Secret;

#[test]
fn diagnostics_never_render_the_inner_value() {
    let secret = Secret::new("correct horse battery staple".to_owned());

    assert_eq!(format!("{secret:?}"), "Secret([REDACTED])");
    assert_eq!(format!("{secret}"), "[REDACTED]");
}

#[test]
fn observation_and_transfer_are_explicit() {
    let secret = Secret::new(vec![1_u8, 2, 3]);

    assert_eq!(secret.expose(), &[1, 2, 3]);
    assert_eq!(secret.into_inner(), vec![1, 2, 3]);
}

#[test]
fn serde_shape_is_identical_to_the_inner_value() {
    let secret = Secret::new(vec!["one".to_owned(), "two".to_owned()]);
    let encoded = serde_json::to_string(&secret).expect("serialize secret wrapper");

    assert_eq!(encoded, r#"["one","two"]"#);

    let decoded: Secret<Vec<String>> =
        serde_json::from_str(&encoded).expect("deserialize transparent wrapper");
    assert_eq!(decoded.expose(), &["one", "two"]);
}
