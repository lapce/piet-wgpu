use ab_glyph::{Font, FontRef};

pub struct FontSource {
    source: font_kit::source::SystemSource,
}

impl FontSource {
    pub fn new() -> Self {
        Self {
            source: font_kit::source::SystemSource::new(),
        }
    }

    pub fn load(&self, family: &piet::FontFamily) -> Result<(), piet::Error> {
        let handle = self
            .source
            .select_best_match(
                &[font_kit::family_name::FamilyName::Title(
                    "Cascadia Code".to_string(),
                )],
                &font_kit::properties::Properties::new(),
            )
            .map_err(|e| piet::Error::NotSupported)?;

        match handle {
            font_kit::handle::Handle::Path { path, font_index } => {
                println!("font path is");
            }
            font_kit::handle::Handle::Memory { bytes, font_index } => {
                let font =
                    FontRef::try_from_slice(&bytes).map_err(|e| piet::Error::NotSupported)?;
                font.glyph_id('a')
                    .with_scale_and_position(12.0, ab_glyph::point(0.0, 0.0));
                println!("font bytes");
            }
        }
        Ok(())
    }
}
