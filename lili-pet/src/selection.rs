use std::path::{Path, PathBuf};

use lili_core::PetId;

use crate::{
    AtlasMetadata, AtlasValidationError, DEFAULT_PET_ID, PetDefinition, PetSummary,
    SPRITE_VERSION_NUMBER, ValidatedPetPackage, discover_pet_packages, validate_discovered_package,
    validation::{read_validated_atlas, validate_atlas_bytes},
};

const FALLBACK_MANIFEST: &str = include_str!("../assets/fallback/pet.json");
const FALLBACK_ATLAS: &[u8] = include_bytes!("../assets/fallback/spritesheet.webp");

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PetAssetSource {
    File(PathBuf),
    Embedded,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AvailablePet {
    definition: PetDefinition,
    atlas: AtlasMetadata,
    source: PetAssetSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedPetAsset {
    bytes: Vec<u8>,
    atlas: AtlasMetadata,
}

impl LoadedPetAsset {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub const fn atlas(&self) -> AtlasMetadata {
        self.atlas
    }
}

impl AvailablePet {
    pub const fn definition(&self) -> &PetDefinition {
        &self.definition
    }

    pub const fn atlas(&self) -> AtlasMetadata {
        self.atlas
    }

    pub const fn source(&self) -> &PetAssetSource {
        &self.source
    }

    pub fn embedded_atlas(&self) -> Option<&'static [u8]> {
        matches!(self.source, PetAssetSource::Embedded).then_some(FALLBACK_ATLAS)
    }

    pub fn load_asset(&self) -> Result<LoadedPetAsset, AtlasValidationError> {
        let (bytes, atlas) = match &self.source {
            PetAssetSource::File(path) => read_validated_atlas(path)?,
            PetAssetSource::Embedded => {
                let bytes = FALLBACK_ATLAS.to_vec();
                let atlas = validate_atlas_bytes(&bytes)?;
                (bytes, atlas)
            }
        };
        Ok(LoadedPetAsset { bytes, atlas })
    }
}

impl From<ValidatedPetPackage> for AvailablePet {
    fn from(package: ValidatedPetPackage) -> Self {
        Self {
            definition: package.definition().clone(),
            atlas: package.atlas(),
            source: PetAssetSource::File(package.atlas_path().to_owned()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogDiagnostic {
    package_dir: Option<PathBuf>,
    message: String,
}

impl CatalogDiagnostic {
    pub fn package_dir(&self) -> Option<&Path> {
        self.package_dir.as_deref()
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PetCatalog {
    active: AvailablePet,
    packages: Vec<AvailablePet>,
    requested_identifier: String,
    diagnostics: Vec<CatalogDiagnostic>,
}

impl Default for PetCatalog {
    fn default() -> Self {
        Self {
            active: fallback_pet(),
            packages: Vec::new(),
            requested_identifier: DEFAULT_PET_ID.to_owned(),
            diagnostics: Vec::new(),
        }
    }
}

impl PetCatalog {
    pub fn load(codex_home: &Path) -> Self {
        Self::load_with_selection(codex_home, None)
    }

    pub fn load_with_selection(codex_home: &Path, selected: Option<&PetId>) -> Self {
        let requested_identifier = selected.map_or(DEFAULT_PET_ID, PetId::as_str).to_owned();
        Self::load_requested(codex_home, requested_identifier, Vec::new())
    }

    fn load_requested(
        codex_home: &Path,
        requested_identifier: String,
        mut diagnostics: Vec<CatalogDiagnostic>,
    ) -> Self {
        let discovered = discover_pet_packages(codex_home);
        diagnostics.extend(discovered.issues().iter().map(|issue| CatalogDiagnostic {
            package_dir: Some(issue.package_dir().to_owned()),
            message: issue.message().to_owned(),
        }));

        let mut packages = Vec::new();
        for package in discovered.packages() {
            match validate_discovered_package(package) {
                Ok(package) => packages.push(AvailablePet::from(package)),
                Err(error) => diagnostics.push(CatalogDiagnostic {
                    package_dir: Some(package.package_dir().to_owned()),
                    message: error.to_string(),
                }),
            }
        }

        let fallback = fallback_pet();
        let active = packages
            .iter()
            .find(|pet| pet.definition().id().as_str() == requested_identifier)
            .cloned()
            .unwrap_or_else(|| {
                if requested_identifier != DEFAULT_PET_ID {
                    diagnostics.push(CatalogDiagnostic {
                        package_dir: None,
                        message: format!(
                            "selected pet `{requested_identifier}` is unavailable; using embedded fallback"
                        ),
                    });
                }
                fallback
            });

        Self {
            active,
            packages,
            requested_identifier,
            diagnostics,
        }
    }

    pub const fn active(&self) -> &AvailablePet {
        &self.active
    }

    pub fn packages(&self) -> &[AvailablePet] {
        &self.packages
    }

    pub fn available_summaries(&self) -> Vec<PetSummary> {
        let mut summaries = vec![PetSummary::from(fallback_pet().definition())];
        for pet in &self.packages {
            let summary = PetSummary::from(pet.definition());
            if let Some(existing) = summaries
                .iter_mut()
                .find(|existing| existing.id == summary.id)
            {
                *existing = summary;
            } else {
                summaries.push(summary);
            }
        }
        summaries.sort_by(|left, right| left.display_name.cmp(&right.display_name));
        summaries
    }

    pub fn requested_identifier(&self) -> &str {
        &self.requested_identifier
    }

    pub fn diagnostics(&self) -> &[CatalogDiagnostic] {
        &self.diagnostics
    }
}

fn fallback_pet() -> AvailablePet {
    let manifest: crate::PetManifest =
        serde_json::from_str(FALLBACK_MANIFEST).expect("embedded fallback manifest must be valid");
    assert_eq!(
        manifest.sprite_version_number(),
        SPRITE_VERSION_NUMBER,
        "embedded fallback must use the v2 manifest"
    );
    AvailablePet {
        definition: PetDefinition {
            id: PetId::parse(manifest.id()).expect("embedded fallback id must be valid"),
            display_name: manifest.display_name().to_owned(),
            description: manifest.description().to_owned(),
        },
        atlas: validate_atlas_bytes(FALLBACK_ATLAS)
            .expect("embedded fallback atlas must pass v2 validation"),
        source: PetAssetSource::Embedded,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use image::GenericImageView;

    use super::*;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "lili-pet-selection-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_user_package_uses_embedded_fallback() {
        let temp = TempDir::new();
        let catalog = PetCatalog::load(&temp.0.join("pets"));
        assert_eq!(catalog.active().definition().id().as_str(), DEFAULT_PET_ID);
        assert_eq!(catalog.active().source(), &PetAssetSource::Embedded);
        assert!(catalog.active().embedded_atlas().is_some());
    }

    #[test]
    fn file_asset_loader_reads_a_bounded_validated_atlas() {
        let temp = TempDir::new();
        let atlas_path = temp.0.join("spritesheet.webp");
        fs::write(&atlas_path, FALLBACK_ATLAS).unwrap();

        let (bytes, metadata) = crate::validation::read_validated_atlas(&atlas_path).unwrap();

        assert_eq!(bytes, FALLBACK_ATLAS);
        assert_eq!(metadata.format(), crate::AtlasFormat::WebP);
        assert_eq!(metadata.width(), crate::ATLAS_WIDTH);
        assert_eq!(metadata.height(), crate::ATLAS_HEIGHT);

        let empty_path = temp.0.join("empty.webp");
        fs::write(&empty_path, []).unwrap();
        assert!(matches!(
            crate::validation::read_validated_atlas(&empty_path),
            Err(AtlasValidationError::EncodedSize(0))
        ));
    }

    #[test]
    fn embedded_fallback_rows_share_one_tabby_palette() {
        const MAX_CHROMATICITY_DRIFT: f64 = 0.03;

        let atlas = image::load_from_memory(FALLBACK_ATLAS)
            .unwrap()
            .into_rgba8();
        let row_palettes = (0..crate::ATLAS_ROWS)
            .map(|row| warm_fur_chromaticity(&atlas, u32::from(row)))
            .collect::<Vec<_>>();
        let standard = [
            median(row_palettes[..9].iter().map(|palette| palette[0])),
            median(row_palettes[..9].iter().map(|palette| palette[1])),
            median(row_palettes[..9].iter().map(|palette| palette[2])),
        ];

        for (row, palette) in row_palettes.iter().enumerate() {
            let drift = palette
                .iter()
                .zip(standard)
                .map(|(value, reference)| (value - reference).powi(2))
                .sum::<f64>()
                .sqrt();
            assert!(
                drift <= MAX_CHROMATICITY_DRIFT,
                "fallback row {row} chromaticity drift {drift:.4} exceeds {MAX_CHROMATICITY_DRIFT:.4}"
            );
        }
    }

    fn warm_fur_chromaticity(atlas: &image::RgbaImage, row: u32) -> [f64; 3] {
        let mut channels = [Vec::new(), Vec::new(), Vec::new()];
        let start_y = row * crate::CELL_HEIGHT;
        for (_, _, pixel) in atlas
            .view(0, start_y, crate::ATLAS_WIDTH, crate::CELL_HEIGHT)
            .pixels()
        {
            let [red, green, blue, alpha] = pixel.0;
            let brightness = (u16::from(red) + u16::from(green) + u16::from(blue)) / 3;
            let span = red.max(green).max(blue) - red.min(green).min(blue);
            if alpha < 128
                || !(40..=210).contains(&brightness)
                || span < 20
                || red < green
                || green < blue
            {
                continue;
            }
            let total = f64::from(red) + f64::from(green) + f64::from(blue);
            channels[0].push(f64::from(red) / total);
            channels[1].push(f64::from(green) / total);
            channels[2].push(f64::from(blue) / total);
        }
        assert!(channels.iter().all(|channel| channel.len() >= 1_000));
        [
            median(channels[0].iter().copied()),
            median(channels[1].iter().copied()),
            median(channels[2].iter().copied()),
        ]
    }

    fn median(values: impl IntoIterator<Item = f64>) -> f64 {
        let mut values = values.into_iter().collect::<Vec<_>>();
        values.sort_by(f64::total_cmp);
        values[values.len() / 2]
    }

    #[test]
    fn fixed_default_package_takes_precedence_over_embedded_fallback() {
        let temp = TempDir::new();
        let package_dir = temp.0.join("pets").join(DEFAULT_PET_ID);
        fs::create_dir_all(&package_dir).unwrap();
        fs::write(package_dir.join("pet.json"), FALLBACK_MANIFEST).unwrap();
        fs::write(package_dir.join("spritesheet.webp"), FALLBACK_ATLAS).unwrap();

        let catalog = PetCatalog::load(&temp.0.join("pets"));

        assert_eq!(catalog.active().definition().id().as_str(), DEFAULT_PET_ID);
        assert!(matches!(catalog.active().source(), PetAssetSource::File(_)));
        assert!(catalog.active().embedded_atlas().is_none());
    }

    #[test]
    fn default_load_ignores_legacy_selection_file() {
        let temp = TempDir::new();
        let legacy = temp.0.join("lili").join("selected-pet.json");
        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        fs::write(&legacy, br#"{"version":1,"petId":"missing"}"#).unwrap();

        let catalog = PetCatalog::load(&temp.0);

        assert_eq!(catalog.requested_identifier(), DEFAULT_PET_ID);
        assert_eq!(catalog.active().source(), &PetAssetSource::Embedded);
    }
}
