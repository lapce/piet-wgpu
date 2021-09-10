pub struct FontSource {
    source: font_kit::source::SystemSource,
}

impl FontSource {
    pub fn new() -> Self {
        Self {
            source: font_kit::source::SystemSource::new(),
        }
    }
}
