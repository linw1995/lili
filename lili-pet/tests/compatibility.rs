use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use image::{ImageFormat, Rgba, RgbaImage};
use lili_pet::{ATLAS_HEIGHT, ATLAS_WIDTH, PetCatalog};
use serde::Deserialize;

const FIXTURES: &[&str] = &[
    include_str!("fixtures/valid-custom.json"),
    include_str!("fixtures/wrong-version.json"),
    include_str!("fixtures/wrong-dimension.json"),
    include_str!("fixtures/malformed-image.json"),
    include_str!("fixtures/opaque-background.json"),
    include_str!("fixtures/escaping-path.json"),
];

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompatibilityFixture {
    name: String,
    package_dir: String,
    manifest: serde_json::Value,
    atlas_kind: AtlasKind,
    expected: ExpectedResult,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum AtlasKind {
    EmbeddedValid,
    WrongDimension,
    Malformed,
    Opaque,
    None,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExpectedResult {
    accepted: bool,
    diagnostic_contains: Option<String>,
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "lili-pet-compatibility-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn golden_package_compatibility_matrix() {
    for fixture in FIXTURES {
        let fixture: CompatibilityFixture = serde_json::from_str(fixture).unwrap();
        run_fixture(&fixture);
    }
}

fn run_fixture(fixture: &CompatibilityFixture) {
    let temp = TempDir::new();
    let package_dir = temp.path().join("pets").join(&fixture.package_dir);
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("pet.json"),
        serde_json::to_vec_pretty(&fixture.manifest).unwrap(),
    )
    .unwrap();
    let spritesheet_path = fixture.manifest["spritesheetPath"].as_str().unwrap();
    write_atlas(&package_dir.join(spritesheet_path), fixture.atlas_kind);

    let catalog = PetCatalog::load(&temp.path().join("pets"));
    let pet_id = fixture.manifest["id"].as_str().unwrap();
    let accepted = catalog
        .packages()
        .iter()
        .any(|pet| pet.definition().id().as_str() == pet_id);
    assert_eq!(accepted, fixture.expected.accepted, "{}", fixture.name);

    if let Some(expected) = &fixture.expected.diagnostic_contains {
        assert!(
            catalog.diagnostics().iter().any(|diagnostic| {
                diagnostic
                    .package_dir()
                    .is_some_and(|path| path.ends_with(&fixture.package_dir))
                    && diagnostic.message().contains(expected)
            }),
            "{} did not report diagnostic containing {expected:?}: {:?}",
            fixture.name,
            catalog.diagnostics()
        );
    } else {
        assert!(
            !catalog.diagnostics().iter().any(|diagnostic| diagnostic
                .package_dir()
                .is_some_and(|path| path.ends_with(&fixture.package_dir))),
            "{} unexpectedly reported diagnostics: {:?}",
            fixture.name,
            catalog.diagnostics()
        );
    }
}

fn write_atlas(path: &Path, kind: AtlasKind) {
    match kind {
        AtlasKind::EmbeddedValid => {
            let catalog = PetCatalog::default();
            fs::write(
                path,
                catalog
                    .active()
                    .embedded_atlas()
                    .expect("fallback must remain embedded"),
            )
            .unwrap();
        }
        AtlasKind::WrongDimension => {
            RgbaImage::from_pixel(1, 1, Rgba([0, 0, 0, 0]))
                .save_with_format(path, ImageFormat::Png)
                .unwrap();
        }
        AtlasKind::Malformed => fs::write(path, b"not-an-image").unwrap(),
        AtlasKind::Opaque => {
            RgbaImage::from_pixel(ATLAS_WIDTH, ATLAS_HEIGHT, Rgba([80, 60, 40, 255]))
                .save_with_format(path, ImageFormat::Png)
                .unwrap();
        }
        AtlasKind::None => {}
    }
}
