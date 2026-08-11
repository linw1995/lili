use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Cursor, Seek},
    path::{Path, PathBuf},
};

use image::{ImageFormat, ImageReader, RgbaImage};
use lili_core::PetId;
use thiserror::Error;

use crate::{
    ATLAS_COLUMNS, ATLAS_HEIGHT, ATLAS_ROWS, ATLAS_WIDTH, CELL_HEIGHT, CELL_WIDTH,
    DiscoveredPackage, NEUTRAL_LOOK_CELL, PetDefinition, STANDARD_ANIMATIONS,
};

const MAX_ENCODED_ATLAS_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AtlasFormat {
    Png,
    WebP,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AtlasMetadata {
    format: AtlasFormat,
    width: u32,
    height: u32,
    encoded_bytes: u64,
}

impl AtlasMetadata {
    pub const fn format(self) -> AtlasFormat {
        self.format
    }

    pub const fn width(self) -> u32 {
        self.width
    }

    pub const fn height(self) -> u32 {
        self.height
    }

    pub const fn encoded_bytes(self) -> u64 {
        self.encoded_bytes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedPetPackage {
    definition: PetDefinition,
    package_dir: PathBuf,
    atlas_path: PathBuf,
    atlas: AtlasMetadata,
}

impl ValidatedPetPackage {
    pub const fn definition(&self) -> &PetDefinition {
        &self.definition
    }

    pub fn package_dir(&self) -> &Path {
        &self.package_dir
    }

    pub fn atlas_path(&self) -> &Path {
        &self.atlas_path
    }

    pub const fn atlas(&self) -> AtlasMetadata {
        self.atlas
    }
}

pub fn validate_discovered_package(
    package: &DiscoveredPackage,
) -> Result<ValidatedPetPackage, AtlasValidationError> {
    let atlas = validate_atlas(package.atlas_path())?;
    let manifest = package.manifest();
    let definition = PetDefinition {
        id: PetId::parse(manifest.id()).expect("discovery validates pet identifiers"),
        display_name: manifest.display_name().to_owned(),
        description: manifest.description().to_owned(),
    };
    Ok(ValidatedPetPackage {
        definition,
        package_dir: package.package_dir().to_owned(),
        atlas_path: package.atlas_path().to_owned(),
        atlas,
    })
}

fn validate_atlas(path: &Path) -> Result<AtlasMetadata, AtlasValidationError> {
    let encoded_bytes = fs::metadata(path)?.len();
    validate_readers(image_reader(path)?, image_reader(path)?, encoded_bytes)
}

fn image_reader(path: &Path) -> Result<ImageReader<BufReader<File>>, AtlasValidationError> {
    Ok(ImageReader::new(BufReader::new(File::open(path)?)).with_guessed_format()?)
}

pub(crate) fn validate_atlas_bytes(
    bytes: &'static [u8],
) -> Result<AtlasMetadata, AtlasValidationError> {
    let encoded_bytes = bytes.len() as u64;
    validate_readers(
        ImageReader::new(Cursor::new(bytes)).with_guessed_format()?,
        ImageReader::new(Cursor::new(bytes)).with_guessed_format()?,
        encoded_bytes,
    )
}

fn validate_readers<R1, R2>(
    metadata_reader: ImageReader<R1>,
    decode_reader: ImageReader<R2>,
    encoded_bytes: u64,
) -> Result<AtlasMetadata, AtlasValidationError>
where
    R1: BufRead + Seek,
    R2: BufRead + Seek,
{
    if encoded_bytes == 0 || encoded_bytes > MAX_ENCODED_ATLAS_BYTES {
        return Err(AtlasValidationError::EncodedSize(encoded_bytes));
    }
    let format = match metadata_reader.format() {
        Some(ImageFormat::Png) => AtlasFormat::Png,
        Some(ImageFormat::WebP) => AtlasFormat::WebP,
        _ => return Err(AtlasValidationError::UnsupportedFormat),
    };
    let (width, height) = metadata_reader.into_dimensions()?;
    if (width, height) != (ATLAS_WIDTH, ATLAS_HEIGHT) {
        return Err(AtlasValidationError::Geometry { width, height });
    }

    let decoded = decode_reader.decode()?;
    if (decoded.width(), decoded.height()) != (ATLAS_WIDTH, ATLAS_HEIGHT) {
        return Err(AtlasValidationError::Geometry {
            width: decoded.width(),
            height: decoded.height(),
        });
    }
    if !decoded.color().has_alpha() {
        return Err(AtlasValidationError::MissingAlphaChannel);
    }
    validate_cells(&decoded.to_rgba8())?;

    Ok(AtlasMetadata {
        format,
        width,
        height,
        encoded_bytes,
    })
}

fn validate_cells(image: &RgbaImage) -> Result<(), AtlasValidationError> {
    let mut visible_cells = [[false; ATLAS_COLUMNS as usize]; ATLAS_ROWS as usize];
    let mut transparent_pixel = false;
    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel.0[3] == 0 {
            transparent_pixel = true;
        } else {
            visible_cells[(y / CELL_HEIGHT) as usize][(x / CELL_WIDTH) as usize] = true;
        }
    }
    if !transparent_pixel {
        return Err(AtlasValidationError::OpaqueSurface);
    }

    for spec in STANDARD_ANIMATIONS {
        for column in 0..ATLAS_COLUMNS {
            let visible = visible_cells[spec.row() as usize][column as usize];
            if (spec.row(), column) == (NEUTRAL_LOOK_CELL.row(), NEUTRAL_LOOK_CELL.column()) {
                if !visible {
                    return Err(AtlasValidationError::EmptyUsedCell {
                        row: spec.row(),
                        column,
                    });
                }
                continue;
            }
            if (column as usize) < spec.frame_count() && !visible {
                return Err(AtlasValidationError::EmptyUsedCell {
                    row: spec.row(),
                    column,
                });
            }
            if (column as usize) >= spec.frame_count() && visible {
                return Err(AtlasValidationError::VisibleUnusedCell {
                    row: spec.row(),
                    column,
                });
            }
        }
    }
    for row in 9..ATLAS_ROWS {
        for column in 0..ATLAS_COLUMNS {
            if !visible_cells[row as usize][column as usize] {
                return Err(AtlasValidationError::EmptyUsedCell { row, column });
            }
        }
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum AtlasValidationError {
    #[error("atlas encoded size {0} is outside the 1..=32 MiB bound")]
    EncodedSize(u64),
    #[error("atlas must be PNG or WebP")]
    UnsupportedFormat,
    #[error("atlas must be 1536x2288, got {width}x{height}")]
    Geometry { width: u32, height: u32 },
    #[error("atlas must have an alpha channel")]
    MissingAlphaChannel,
    #[error("atlas must contain transparent background pixels")]
    OpaqueSurface,
    #[error("required atlas cell {row}:{column} is empty")]
    EmptyUsedCell { row: u8, column: u8 },
    #[error("unused atlas cell {row}:{column} is not transparent")]
    VisibleUnusedCell { row: u8, column: u8 },
    #[error("atlas I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("atlas decode failed: {0}")]
    Image(#[from] image::ImageError),
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use image::{
        ImageEncoder,
        codecs::{png::PngEncoder, webp::WebPEncoder},
    };

    use super::*;

    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

    fn fixture_path(extension: &str) -> PathBuf {
        let sequence = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "lili-atlas-{}-{sequence}.{extension}",
            std::process::id()
        ))
    }

    fn valid_pixels() -> RgbaImage {
        let mut image = RgbaImage::new(ATLAS_WIDTH, ATLAS_HEIGHT);
        for spec in STANDARD_ANIMATIONS {
            for column in 0..spec.frame_count() as u32 {
                image.put_pixel(
                    column * CELL_WIDTH + CELL_WIDTH / 2,
                    u32::from(spec.row()) * CELL_HEIGHT + CELL_HEIGHT / 2,
                    image::Rgba([80, 60, 40, 255]),
                );
            }
        }
        image.put_pixel(
            u32::from(NEUTRAL_LOOK_CELL.column()) * CELL_WIDTH + CELL_WIDTH / 2,
            u32::from(NEUTRAL_LOOK_CELL.row()) * CELL_HEIGHT + CELL_HEIGHT / 2,
            image::Rgba([80, 60, 40, 255]),
        );
        for row in 9..ATLAS_ROWS as u32 {
            for column in 0..ATLAS_COLUMNS as u32 {
                image.put_pixel(
                    column * CELL_WIDTH + CELL_WIDTH / 2,
                    row * CELL_HEIGHT + CELL_HEIGHT / 2,
                    image::Rgba([80, 60, 40, 255]),
                );
            }
        }
        image
    }

    fn write_png(path: &Path, image: &RgbaImage) {
        let file = File::create(path).unwrap();
        PngEncoder::new(file)
            .write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
    }

    fn write_webp(path: &Path, image: &RgbaImage) {
        let file = File::create(path).unwrap();
        WebPEncoder::new_lossless(file)
            .write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
    }

    #[test]
    fn validates_exact_transparent_png() {
        let path = fixture_path("png");
        write_png(&path, &valid_pixels());
        let metadata = validate_atlas(&path).unwrap();
        assert_eq!(metadata.format(), AtlasFormat::Png);
        assert_eq!((metadata.width(), metadata.height()), (1536, 2288));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn validates_exact_transparent_webp() {
        let path = fixture_path("webp");
        write_webp(&path, &valid_pixels());
        let metadata = validate_atlas(&path).unwrap();
        assert_eq!(metadata.format(), AtlasFormat::WebP);
        assert_eq!((metadata.width(), metadata.height()), (1536, 2288));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_wrong_geometry_before_surface_validation() {
        let path = fixture_path("png");
        write_png(&path, &RgbaImage::new(8, 11));
        assert!(matches!(
            validate_atlas(&path),
            Err(AtlasValidationError::Geometry {
                width: 8,
                height: 11
            })
        ));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_opaque_background() {
        let path = fixture_path("png");
        let image = RgbaImage::from_pixel(ATLAS_WIDTH, ATLAS_HEIGHT, image::Rgba([0, 0, 0, 255]));
        write_png(&path, &image);
        assert!(matches!(
            validate_atlas(&path),
            Err(AtlasValidationError::OpaqueSurface)
        ));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_visible_unused_cell() {
        let path = fixture_path("png");
        let mut image = valid_pixels();
        image.put_pixel(7 * CELL_WIDTH, 0, image::Rgba([0, 0, 0, 255]));
        write_png(&path, &image);
        assert!(matches!(
            validate_atlas(&path),
            Err(AtlasValidationError::VisibleUnusedCell { row: 0, column: 7 })
        ));
        fs::remove_file(path).unwrap();
    }
}
