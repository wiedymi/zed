use std::{borrow::Cow, collections::HashMap, sync::Arc};

use anyhow::{Context as _, Result, bail};
use etagere::{BucketedAtlasAllocator, size2};
use gpui::{
    AtlasKey, AtlasTextureId, AtlasTextureKind, AtlasTile, Bounds, DevicePixels, PlatformAtlas,
    Point, Size,
};
use parking_lot::Mutex;

const DEFAULT_TEXTURE_SIZE: i32 = 1024;

/// CPU-backed atlas shared by text/image rasterization and Native Drawing.
pub(crate) struct OhosAtlas(Mutex<AtlasState>);

#[derive(Clone)]
pub(crate) struct TilePixels {
    pub(crate) kind: AtlasTextureKind,
    pub(crate) size: Size<DevicePixels>,
    pub(crate) revision: u64,
    pub(crate) bytes: Arc<[u8]>,
}

#[derive(Default)]
struct AtlasState {
    tiles_by_key: HashMap<AtlasKey, AtlasTile>,
    textures: Vec<Texture>,
    next_revision: u64,
}

struct Texture {
    kind: AtlasTextureKind,
    allocator: BucketedAtlasAllocator,
    size: Size<DevicePixels>,
    bytes: Vec<u8>,
    tiles: HashMap<u32, TileData>,
    live_tiles: u32,
}

struct TileData {
    revision: u64,
    bytes: Arc<[u8]>,
}

impl OhosAtlas {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(AtlasState::default())))
    }

    pub(crate) fn tile_pixels(&self, tile: AtlasTile) -> Result<TilePixels> {
        let state = self.0.lock();
        let texture = state
            .textures
            .get(tile.texture_id.index as usize)
            .context("atlas texture does not exist")?;
        if texture.kind != tile.texture_id.kind {
            bail!("atlas texture kind does not match tile");
        }
        let tile_data = texture
            .tiles
            .get(&tile.tile_id.0)
            .context("atlas tile pixels do not exist")?;

        Ok(TilePixels {
            kind: texture.kind,
            size: tile.bounds.size,
            revision: tile_data.revision,
            bytes: tile_data.bytes.clone(),
        })
    }
}

impl PlatformAtlas for OhosAtlas {
    fn get_or_insert_with<'a>(
        &self,
        key: &AtlasKey,
        build: &mut dyn FnMut() -> Result<Option<(Size<DevicePixels>, Cow<'a, [u8]>)>>,
    ) -> Result<Option<AtlasTile>> {
        let mut state = self.0.lock();
        if let Some(tile) = state.tiles_by_key.get(key) {
            return Ok(Some(*tile));
        }

        let Some((size, bytes)) = build()? else {
            return Ok(None);
        };
        let kind = key.texture_kind();
        let tile = state.allocate(size, kind)?;
        state.upload(tile, &bytes)?;
        state.tiles_by_key.insert(key.clone(), tile);
        Ok(Some(tile))
    }

    fn remove(&self, key: &AtlasKey) {
        let mut state = self.0.lock();
        let Some(tile) = state.tiles_by_key.remove(key) else {
            return;
        };
        let Some(texture) = state.textures.get_mut(tile.texture_id.index as usize) else {
            return;
        };
        texture.allocator.deallocate(tile.tile_id.into());
        texture.tiles.remove(&tile.tile_id.0);
        texture.live_tiles = texture.live_tiles.saturating_sub(1);
    }
}

impl AtlasState {
    fn allocate(&mut self, size: Size<DevicePixels>, kind: AtlasTextureKind) -> Result<AtlasTile> {
        for (index, texture) in self.textures.iter_mut().enumerate().rev() {
            if texture.kind == kind
                && let Some(tile) = texture.allocate(index, size)
            {
                return Ok(tile);
            }
        }

        let width = size.width.0.max(DEFAULT_TEXTURE_SIZE);
        let height = size.height.0.max(DEFAULT_TEXTURE_SIZE);
        if width <= 0 || height <= 0 {
            bail!("cannot allocate an empty atlas tile");
        }
        let byte_count = usize::try_from(width)?
            .checked_mul(usize::try_from(height)?)
            .and_then(|pixels| pixels.checked_mul(channels(kind)))
            .context("atlas texture byte size overflowed")?;
        self.textures.push(Texture {
            kind,
            allocator: BucketedAtlasAllocator::new(size2(width, height)),
            size: Size::new(DevicePixels(width), DevicePixels(height)),
            bytes: vec![0; byte_count],
            tiles: HashMap::new(),
            live_tiles: 0,
        });
        let index = self.textures.len() - 1;
        self.textures[index]
            .allocate(index, size)
            .context("new atlas texture could not fit requested tile")
    }

    fn upload(&mut self, tile: AtlasTile, bytes: &[u8]) -> Result<()> {
        let revision = self.next_revision;
        self.next_revision = self.next_revision.wrapping_add(1);
        let texture = self
            .textures
            .get_mut(tile.texture_id.index as usize)
            .context("atlas texture disappeared before upload")?;
        let channels = channels(texture.kind);
        let width = usize::try_from(tile.bounds.size.width.0)?;
        let height = usize::try_from(tile.bounds.size.height.0)?;
        let expected_length = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(channels))
            .context("atlas tile byte size overflowed")?;
        if bytes.len() != expected_length {
            bail!(
                "atlas upload has {} bytes, expected {expected_length}",
                bytes.len()
            );
        }

        let texture_width = usize::try_from(texture.size.width.0)?;
        let destination_x = usize::try_from(tile.bounds.origin.x.0)?;
        let destination_y = usize::try_from(tile.bounds.origin.y.0)?;
        for row in 0..height {
            let source_start = row * width * channels;
            let destination_start =
                ((destination_y + row) * texture_width + destination_x) * channels;
            texture.bytes[destination_start..destination_start + width * channels]
                .copy_from_slice(&bytes[source_start..source_start + width * channels]);
        }
        texture.tiles.insert(
            tile.tile_id.0,
            TileData {
                revision,
                bytes: Arc::from(bytes),
            },
        );
        Ok(())
    }
}

impl Texture {
    fn allocate(&mut self, index: usize, size: Size<DevicePixels>) -> Option<AtlasTile> {
        let allocation = self
            .allocator
            .allocate(size2(size.width.0, size.height.0))?;
        self.live_tiles += 1;
        Some(AtlasTile {
            texture_id: AtlasTextureId {
                index: u32::try_from(index).ok()?,
                kind: self.kind,
            },
            tile_id: allocation.id.into(),
            padding: 0,
            bounds: Bounds {
                origin: Point::new(
                    DevicePixels(allocation.rectangle.min.x),
                    DevicePixels(allocation.rectangle.min.y),
                ),
                size,
            },
        })
    }
}

fn channels(kind: AtlasTextureKind) -> usize {
    match kind {
        AtlasTextureKind::Monochrome => 1,
        AtlasTextureKind::Subpixel | AtlasTextureKind::Polychrome => 4,
    }
}
