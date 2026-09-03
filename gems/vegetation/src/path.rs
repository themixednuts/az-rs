pub fn non_empty_path(value: Option<&str>) -> Option<&str> {
    value.and_then(|path| {
        let trimmed = path.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    })
}
