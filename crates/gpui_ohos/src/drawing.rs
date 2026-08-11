use std::{cell::RefCell, collections::HashMap, ffi::c_void, ptr::NonNull, sync::Arc};
#[cfg(debug_assertions)]
use std::{
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, anyhow, bail};
use gpui::{
    AtlasTextureKind, BackgroundKind, BorderStyle, Bounds, ColorSpace, Corners, Edges,
    LinearColorStop, MonochromeSprite, Path, PolychromeSprite, PrimitiveBatch, Quad, Rgba,
    ScaledPixels, Scene, Shadow, SubpixelSprite, Underline,
};

use crate::atlas::{OhosAtlas, TilePixels};
use ohos_sys::drawing::{
    bitmap, brush, canvas,
    canvas::OH_Drawing_CanvasClipOp,
    filter, gpu_context, mask_filter, matrix, path, path_effect, pen, point, rect, round_rect,
    sampling_options,
    sampling_options::{OH_Drawing_FilterMode, OH_Drawing_MipmapMode},
    shader_effect, surface,
    types::{
        OH_Drawing_AlphaFormat, OH_Drawing_Bitmap, OH_Drawing_BlendMode, OH_Drawing_Brush,
        OH_Drawing_Canvas, OH_Drawing_ColorFormat, OH_Drawing_Corner_Radii, OH_Drawing_Filter,
        OH_Drawing_GpuContext, OH_Drawing_Image_Info, OH_Drawing_MaskFilter, OH_Drawing_Matrix,
        OH_Drawing_Path, OH_Drawing_PathEffect, OH_Drawing_Pen, OH_Drawing_Point, OH_Drawing_Rect,
        OH_Drawing_RoundRect, OH_Drawing_SamplingOptions, OH_Drawing_ShaderEffect,
        OH_Drawing_Surface,
    },
};
use ohos_sys::native_window::{
    NativeWindowOperation, OH_NativeWindow_NativeWindowHandleOpt,
    OH_NativeWindow_NativeWindowSetScalingModeV2, OHScalingModeV2,
};

#[derive(Clone, Copy)]
enum NativeObjectKind {
    GpuContext,
    Surface,
    Bitmap,
    Sampling,
    Filter,
    MaskFilter,
    Brush,
    Point,
    Matrix,
    Shader,
    Rect,
    RoundRect,
    Pen,
    PathEffect,
    Path,
}

#[cfg(debug_assertions)]
impl NativeObjectKind {
    const ALL: [Self; 15] = [
        Self::GpuContext,
        Self::Surface,
        Self::Bitmap,
        Self::Sampling,
        Self::Filter,
        Self::MaskFilter,
        Self::Brush,
        Self::Point,
        Self::Matrix,
        Self::Shader,
        Self::Rect,
        Self::RoundRect,
        Self::Pen,
        Self::PathEffect,
        Self::Path,
    ];

    fn index(self) -> usize {
        self as usize
    }

    fn name(self) -> &'static str {
        match self {
            Self::GpuContext => "gpu-context",
            Self::Surface => "surface",
            Self::Bitmap => "bitmap",
            Self::Sampling => "sampling",
            Self::Filter => "filter",
            Self::MaskFilter => "mask-filter",
            Self::Brush => "brush",
            Self::Point => "point",
            Self::Matrix => "matrix",
            Self::Shader => "shader",
            Self::Rect => "rect",
            Self::RoundRect => "round-rect",
            Self::Pen => "pen",
            Self::PathEffect => "path-effect",
            Self::Path => "path",
        }
    }
}

#[cfg(debug_assertions)]
struct NativeObjectCounter {
    live: AtomicUsize,
    peak: AtomicUsize,
    created: AtomicUsize,
}

#[cfg(debug_assertions)]
fn native_object_counters() -> &'static [NativeObjectCounter; NativeObjectKind::ALL.len()] {
    static COUNTERS: OnceLock<[NativeObjectCounter; NativeObjectKind::ALL.len()]> = OnceLock::new();
    COUNTERS.get_or_init(|| {
        std::array::from_fn(|_| NativeObjectCounter {
            live: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            created: AtomicUsize::new(0),
        })
    })
}

#[cfg(debug_assertions)]
fn track_native_object<T>(kind: NativeObjectKind, value: T) -> T {
    let counter = &native_object_counters()[kind.index()];
    let live = counter.live.fetch_add(1, Ordering::Relaxed) + 1;
    counter.created.fetch_add(1, Ordering::Relaxed);
    counter.peak.fetch_max(live, Ordering::Relaxed);
    value
}

#[cfg(not(debug_assertions))]
fn track_native_object<T>(_kind: NativeObjectKind, value: T) -> T {
    value
}

#[cfg(debug_assertions)]
fn release_native_object(kind: NativeObjectKind) {
    let counter = &native_object_counters()[kind.index()];
    if counter
        .live
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |live| {
            live.checked_sub(1)
        })
        .is_err()
    {
        crate::log_message(
            crate::LogLevel::Error,
            format!(
                "Native Drawing {} lifetime counter underflowed",
                kind.name()
            ),
        );
    }
}

#[cfg(not(debug_assertions))]
fn release_native_object(_kind: NativeObjectKind) {}

#[cfg(debug_assertions)]
fn native_object_profile() -> String {
    NativeObjectKind::ALL
        .into_iter()
        .filter_map(|kind| {
            let counter = &native_object_counters()[kind.index()];
            let created = counter.created.load(Ordering::Relaxed);
            (created > 0).then(|| {
                format!(
                    "{}={}/{}/{}",
                    kind.name(),
                    counter.live.load(Ordering::Relaxed),
                    counter.peak.load(Ordering::Relaxed),
                    created,
                )
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub struct NativeDrawingSurface {
    gpu_context: Option<NonNull<OH_Drawing_GpuContext>>,
    surface: Option<NonNull<OH_Drawing_Surface>>,
    window: NonNull<c_void>,
    width: u32,
    height: u32,
    sampling: OwnedSamplingOptions,
    sprite_bitmaps: RefCell<HashMap<SpriteBitmapKey, CachedBitmap>>,
    frame_id: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SpriteBitmapKey {
    texture_index: u32,
    tile_id: u32,
    revision: u64,
    style: SpriteBitmapStyle,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum SpriteBitmapStyle {
    Monochrome {
        red: u32,
        green: u32,
        blue: u32,
        alpha: u32,
    },
    Subpixel {
        red: u32,
        green: u32,
        blue: u32,
        alpha: u32,
    },
    Polychrome {
        grayscale: bool,
        opacity: u32,
    },
}

struct CachedBitmap {
    bitmap: OwnedBitmap,
    last_used_frame: u64,
}

impl NativeDrawingSurface {
    /// Creates an on-screen Native Drawing surface for an XComponent window.
    ///
    /// # Safety
    ///
    /// `window` must be the live native window supplied by the XComponent
    /// surface callback. The resulting value must be dropped before that
    /// native window is destroyed.
    pub unsafe fn new(window: NonNull<c_void>, width: u32, height: u32) -> Result<Self> {
        let image_info = configure_native_window(window, width, height)?;
        let sampling = OwnedSamplingOptions::new()?;

        let (gpu_context, native_surface) = create_gpu_surface(window, image_info)?;

        Ok(Self {
            gpu_context: Some(gpu_context),
            surface: Some(native_surface),
            window,
            width,
            height,
            sampling,
            sprite_bitmaps: RefCell::new(HashMap::new()),
            frame_id: 0,
        })
    }

    /// Rebinds the on-screen surface to the XComponent's new buffer geometry.
    /// The GPU context remains alive across interactive PC-window resizes.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if self.size() == (width, height) && self.surface.is_some() {
            return Ok(());
        }

        self.replace_surface(width, height)
    }

    /// Recreates all GPU resources after another native window was active.
    pub fn rebind(&mut self) -> Result<()> {
        self.destroy_gpu_surface();
        let image_info = configure_native_window(self.window, self.width, self.height)?;
        let (gpu_context, native_surface) = create_gpu_surface(self.window, image_info)?;
        self.gpu_context = Some(gpu_context);
        self.surface = Some(native_surface);
        Ok(())
    }

    fn replace_surface(&mut self, width: u32, height: u32) -> Result<()> {
        if let Some(previous) = self.surface.take() {
            // Native Drawing's on-screen wrappers ultimately own the EGLSurface
            // associated with this NativeWindow. Two wrappers cannot overlap:
            // destroying the older wrapper after creating its replacement also
            // destroys the replacement's EGLSurface (EGL_BAD_SURFACE on flush).
            // SAFETY: `previous` was uniquely owned and has been removed from
            // the struct, so Drop cannot destroy it again if rebinding fails.
            unsafe { surface::OH_Drawing_SurfaceDestroy(previous.as_ptr()) };
            release_native_object(NativeObjectKind::Surface);
        }

        let image_info = configure_native_window(self.window, width, height)?;
        let gpu_context = self
            .gpu_context
            .context("Native Drawing GPU context is unavailable")?;
        // SAFETY: The GPU context and NativeWindow remain live for the whole
        // XComponent surface lifetime, and no other on-screen wrapper exists.
        let replacement = NonNull::new(unsafe {
            surface::OH_Drawing_SurfaceCreateOnScreen(
                gpu_context.as_ptr(),
                image_info,
                self.window.as_ptr(),
            )
        })
        .ok_or_else(|| anyhow!("OH_Drawing_SurfaceCreateOnScreen returned null during resize"))?;
        track_native_object(NativeObjectKind::Surface, ());
        self.surface = Some(replacement);
        self.width = width;
        self.height = height;
        Ok(())
    }

    fn destroy_gpu_surface(&mut self) {
        // SAFETY: Both handles are uniquely owned by this value. Native
        // Drawing requires any live surface to be destroyed before its
        // context.
        unsafe {
            if let Some(native_surface) = self.surface.take() {
                surface::OH_Drawing_SurfaceDestroy(native_surface.as_ptr());
                release_native_object(NativeObjectKind::Surface);
            }
            if let Some(gpu_context) = self.gpu_context.take() {
                gpu_context::OH_Drawing_GpuContextDestroy(gpu_context.as_ptr());
                release_native_object(NativeObjectKind::GpuContext);
            }
        }
    }

    pub fn window(&self) -> NonNull<c_void> {
        self.window
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn is_available(&self) -> bool {
        self.surface.is_some()
    }

    pub fn clear(&mut self, color: u32) -> Result<()> {
        let canvas = self.canvas()?;
        // SAFETY: The canvas is borrowed from the live surface and the ARGB
        // color is passed by value.
        unsafe { canvas::OH_Drawing_CanvasClear(canvas.as_ptr(), color) };
        self.flush()
    }

    pub(crate) fn draw_scene(&mut self, scene: &Scene, atlas: &OhosAtlas) -> Result<()> {
        self.frame_id = self.frame_id.wrapping_add(1);
        #[cfg(debug_assertions)]
        let started_at = Instant::now();
        let native_canvas = self.canvas()?;
        // GPUI scenes are rebuilt completely, so begin with a transparent
        // target and draw the scene's ordered primitive batches.
        // SAFETY: `native_canvas` is borrowed from the live on-screen surface.
        unsafe { canvas::OH_Drawing_CanvasClear(native_canvas.as_ptr(), 0x00000000) };

        for batch in scene.batches() {
            match batch {
                PrimitiveBatch::Shadows(range) => {
                    for shadow in &scene.shadows[range] {
                        self.draw_shadow(native_canvas, shadow)?;
                    }
                }
                PrimitiveBatch::Quads(range) => {
                    for quad in &scene.quads[range] {
                        self.draw_quad(native_canvas, quad)?;
                    }
                }
                PrimitiveBatch::Paths(range) => {
                    for scene_path in &scene.paths[range] {
                        self.draw_path(native_canvas, scene_path)?;
                    }
                }
                PrimitiveBatch::Underlines(range) => {
                    for underline in &scene.underlines[range] {
                        self.draw_underline(native_canvas, underline)?;
                    }
                }
                PrimitiveBatch::MonochromeSprites { range, .. } => {
                    for sprite in &scene.monochrome_sprites[range] {
                        self.draw_monochrome_sprite(native_canvas, sprite, atlas)?;
                    }
                }
                PrimitiveBatch::SubpixelSprites { range, .. } => {
                    for sprite in &scene.subpixel_sprites[range] {
                        self.draw_subpixel_sprite(native_canvas, sprite, atlas)?;
                    }
                }
                PrimitiveBatch::PolychromeSprites { range, .. } => {
                    for sprite in &scene.polychrome_sprites[range] {
                        self.draw_polychrome_sprite(native_canvas, sprite, atlas)?;
                    }
                }
                PrimitiveBatch::Surfaces(_) => {}
            }
        }
        #[cfg(debug_assertions)]
        let drawn_at = Instant::now();

        self.flush()?;
        #[cfg(debug_assertions)]
        {
            let elapsed = started_at.elapsed();
            if elapsed >= Duration::from_millis(33) && self.frame_id.is_multiple_of(120) {
                crate::log_message(
                    crate::LogLevel::Debug,
                    format!(
                        "slow Native Drawing frame: total={elapsed:?}, draw={:?}, flush={:?}",
                        drawn_at.duration_since(started_at),
                        elapsed.saturating_sub(drawn_at.duration_since(started_at)),
                    ),
                );
            }
        }
        if self.frame_id.is_multiple_of(120) {
            let oldest_frame = self.frame_id.saturating_sub(120);
            self.sprite_bitmaps
                .borrow_mut()
                .retain(|_, bitmap| bitmap.last_used_frame >= oldest_frame);
            #[cfg(debug_assertions)]
            {
                let cached_bitmaps = self.sprite_bitmaps.borrow();
                let cached_bitmap_bytes = cached_bitmaps.values().fold(0usize, |total, bitmap| {
                    total.saturating_add(bitmap.bitmap._pixels.len())
                });
                let path_vertices = scene.paths.iter().fold(0usize, |total, path| {
                    total.saturating_add(path.vertices.len())
                });
                crate::log_message(
                    crate::LogLevel::Debug,
                    format!(
                        "Native Drawing profile frame={}: shadows={} quads={} paths={}/{}-vertices underlines={} sprites={}/{}/{} cached-bitmaps={}/{}-bytes native(live/peak/created): {}",
                        self.frame_id,
                        scene.shadows.len(),
                        scene.quads.len(),
                        scene.paths.len(),
                        path_vertices,
                        scene.underlines.len(),
                        scene.monochrome_sprites.len(),
                        scene.subpixel_sprites.len(),
                        scene.polychrome_sprites.len(),
                        cached_bitmaps.len(),
                        cached_bitmap_bytes,
                        native_object_profile(),
                    ),
                );
            }
        }
        Ok(())
    }

    fn canvas(&self) -> Result<NonNull<OH_Drawing_Canvas>> {
        let native_surface = self
            .surface
            .context("Native Drawing surface is unavailable")?;
        // SAFETY: `native_surface` is live for the lifetime of `self`.
        NonNull::new(unsafe { surface::OH_Drawing_SurfaceGetCanvas(native_surface.as_ptr()) })
            .ok_or_else(|| anyhow!("OH_Drawing_SurfaceGetCanvas returned null"))
    }

    fn flush(&self) -> Result<()> {
        let native_surface = self
            .surface
            .context("Native Drawing surface is unavailable")?;
        // SAFETY: `native_surface` is live and owned by this value.
        unsafe { surface::OH_Drawing_SurfaceFlush(native_surface.as_ptr()) }
            .map_err(|error| anyhow!("OH_Drawing_SurfaceFlush failed: {error:?}"))
    }

    fn draw_quad(&self, native_canvas: NonNull<OH_Drawing_Canvas>, quad: &Quad) -> Result<()> {
        let outer_rect = OwnedRect::from_bounds(quad.bounds)?;
        let outer_round_rect = OwnedRoundRect::new(&outer_rect, quad.corner_radii)?;
        let content_mask = OwnedRect::from_bounds(quad.content_mask.bounds)?;

        match quad.background.kind() {
            BackgroundKind::Solid(color) => {
                let native_brush = OwnedBrush::new(argb(color.to_rgb()))?;
                self.draw_brush_in_round_shape(
                    native_canvas,
                    &content_mask,
                    &outer_rect,
                    &outer_round_rect,
                    &native_brush,
                );
            }
            BackgroundKind::LinearGradient {
                angle,
                color_space,
                colors,
            } => {
                let native_brush =
                    OwnedBrush::linear_gradient(quad.bounds, angle, color_space, colors)?;
                self.draw_brush_in_round_shape(
                    native_canvas,
                    &content_mask,
                    &outer_rect,
                    &outer_round_rect,
                    &native_brush,
                );
            }
            BackgroundKind::PatternSlash {
                color,
                width,
                interval,
            } => self.draw_slash_pattern(
                native_canvas,
                quad.bounds,
                &content_mask,
                &outer_rect,
                &outer_round_rect,
                color.to_rgb(),
                width,
                interval,
            )?,
            BackgroundKind::Checkerboard { color, size } => self.draw_checkerboard(
                native_canvas,
                quad.bounds,
                &content_mask,
                &outer_rect,
                &outer_round_rect,
                color.to_rgb(),
                size,
            )?,
        }

        if !quad.border_color.is_transparent() && border_is_visible(quad.border_widths) {
            match quad.border_style {
                BorderStyle::Solid => self.draw_solid_border(
                    native_canvas,
                    quad,
                    &content_mask,
                    &outer_rect,
                    &outer_round_rect,
                )?,
                BorderStyle::Dashed => {
                    self.draw_dashed_border(native_canvas, quad, &content_mask)?
                }
            }
        }

        Ok(())
    }

    fn draw_brush_in_round_shape(
        &self,
        native_canvas: NonNull<OH_Drawing_Canvas>,
        content_mask: &OwnedRect,
        shape_rect: &OwnedRect,
        shape_round_rect: &OwnedRoundRect,
        native_brush: &OwnedBrush,
    ) {
        // SAFETY: All handles remain live through this balanced
        // save/clip/draw/restore sequence.
        unsafe {
            canvas::OH_Drawing_CanvasSave(native_canvas.as_ptr());
            clip_rect(native_canvas, content_mask);
            draw_round_rect(native_canvas, shape_rect, shape_round_rect, native_brush);
            canvas::OH_Drawing_CanvasRestore(native_canvas.as_ptr());
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_slash_pattern(
        &self,
        native_canvas: NonNull<OH_Drawing_Canvas>,
        bounds: Bounds<ScaledPixels>,
        content_mask: &OwnedRect,
        shape_rect: &OwnedRect,
        shape_round_rect: &OwnedRoundRect,
        color: Rgba,
        width: f32,
        interval: f32,
    ) -> Result<()> {
        if width <= 0.0 {
            return Ok(());
        }
        let diagonal_scale = std::f32::consts::FRAC_1_SQRT_2;
        let native_path =
            OwnedPath::slash_pattern(bounds, (width + interval.max(0.0)) * diagonal_scale)?;
        let native_pen = OwnedPen::new(argb(color), width * diagonal_scale, None)?;
        // SAFETY: All handles remain live through the balanced canvas stack.
        unsafe {
            canvas::OH_Drawing_CanvasSave(native_canvas.as_ptr());
            clip_rect(native_canvas, content_mask);
            clip_round_rect(
                native_canvas,
                shape_rect,
                shape_round_rect,
                OH_Drawing_CanvasClipOp::INTERSECT,
            );
            canvas::OH_Drawing_CanvasAttachPen(native_canvas.as_ptr(), native_pen.0.as_ptr());
            canvas::OH_Drawing_CanvasDrawPath(native_canvas.as_ptr(), native_path.0.as_ptr());
            canvas::OH_Drawing_CanvasDetachPen(native_canvas.as_ptr());
            canvas::OH_Drawing_CanvasRestore(native_canvas.as_ptr());
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_checkerboard(
        &self,
        native_canvas: NonNull<OH_Drawing_Canvas>,
        bounds: Bounds<ScaledPixels>,
        content_mask: &OwnedRect,
        shape_rect: &OwnedRect,
        shape_round_rect: &OwnedRoundRect,
        color: Rgba,
        size: f32,
    ) -> Result<()> {
        if size <= 0.0 {
            return Ok(());
        }
        let native_path = OwnedPath::checkerboard(bounds, size)?;
        let native_brush = OwnedBrush::new(argb(color))?;
        // SAFETY: All handles remain live through the balanced canvas stack.
        unsafe {
            canvas::OH_Drawing_CanvasSave(native_canvas.as_ptr());
            clip_rect(native_canvas, content_mask);
            clip_round_rect(
                native_canvas,
                shape_rect,
                shape_round_rect,
                OH_Drawing_CanvasClipOp::INTERSECT,
            );
            canvas::OH_Drawing_CanvasAttachBrush(
                native_canvas.as_ptr(),
                native_brush.native.as_ptr(),
            );
            canvas::OH_Drawing_CanvasDrawPath(native_canvas.as_ptr(), native_path.0.as_ptr());
            canvas::OH_Drawing_CanvasDetachBrush(native_canvas.as_ptr());
            canvas::OH_Drawing_CanvasRestore(native_canvas.as_ptr());
        }
        Ok(())
    }

    fn draw_solid_border(
        &self,
        native_canvas: NonNull<OH_Drawing_Canvas>,
        quad: &Quad,
        content_mask: &OwnedRect,
        outer_rect: &OwnedRect,
        outer_round_rect: &OwnedRoundRect,
    ) -> Result<()> {
        let inner_bounds = inset_bounds(quad.bounds, quad.border_widths);
        let inner_rect = OwnedRect::from_bounds(inner_bounds)?;
        let inner_round_rect = OwnedRoundRect::new_elliptical(
            &inner_rect,
            inset_elliptical_radii(quad.corner_radii, quad.border_widths),
        )?;
        let native_brush = OwnedBrush::new(argb(quad.border_color.to_rgb()))?;

        // SAFETY: All handles remain live through this balanced clip stack.
        unsafe {
            canvas::OH_Drawing_CanvasSave(native_canvas.as_ptr());
            clip_rect(native_canvas, content_mask);
            clip_round_rect(
                native_canvas,
                outer_rect,
                outer_round_rect,
                OH_Drawing_CanvasClipOp::INTERSECT,
            );
            clip_round_rect(
                native_canvas,
                &inner_rect,
                &inner_round_rect,
                OH_Drawing_CanvasClipOp::DIFFERENCE,
            );
            draw_round_rect(native_canvas, outer_rect, outer_round_rect, &native_brush);
            canvas::OH_Drawing_CanvasRestore(native_canvas.as_ptr());
        }
        Ok(())
    }

    fn draw_dashed_border(
        &self,
        native_canvas: NonNull<OH_Drawing_Canvas>,
        quad: &Quad,
        content_mask: &OwnedRect,
    ) -> Result<()> {
        let width = maximum_border_width(quad.border_widths).max(1.0);
        let center_bounds = inset_bounds(
            quad.bounds,
            Edges {
                top: ScaledPixels(width * 0.5),
                right: ScaledPixels(width * 0.5),
                bottom: ScaledPixels(width * 0.5),
                left: ScaledPixels(width * 0.5),
            },
        );
        let center_rect = OwnedRect::from_bounds(center_bounds)?;
        let center_radii = Corners {
            top_left: ScaledPixels((quad.corner_radii.top_left.0 - width * 0.5).max(0.0)),
            top_right: ScaledPixels((quad.corner_radii.top_right.0 - width * 0.5).max(0.0)),
            bottom_right: ScaledPixels((quad.corner_radii.bottom_right.0 - width * 0.5).max(0.0)),
            bottom_left: ScaledPixels((quad.corner_radii.bottom_left.0 - width * 0.5).max(0.0)),
        };
        let center_round_rect = OwnedRoundRect::new(&center_rect, center_radii)?;
        let dash_effect = OwnedPathEffect::dash([width * 2.0, width])?;
        let native_pen =
            OwnedPen::new(argb(quad.border_color.to_rgb()), width, Some(&dash_effect))?;

        // Dashed borders are stroked along the rounded centerline. Clip them
        // to the actual per-edge border ring so zero/narrow edges remain exact.
        let outer_rect = OwnedRect::from_bounds(quad.bounds)?;
        let outer_round_rect = OwnedRoundRect::new(&outer_rect, quad.corner_radii)?;
        let inner_rect = OwnedRect::from_bounds(inset_bounds(quad.bounds, quad.border_widths))?;
        let inner_round_rect = OwnedRoundRect::new_elliptical(
            &inner_rect,
            inset_elliptical_radii(quad.corner_radii, quad.border_widths),
        )?;

        // SAFETY: All handles remain live through the balanced canvas stack.
        unsafe {
            canvas::OH_Drawing_CanvasSave(native_canvas.as_ptr());
            clip_rect(native_canvas, content_mask);
            clip_round_rect(
                native_canvas,
                &outer_rect,
                &outer_round_rect,
                OH_Drawing_CanvasClipOp::INTERSECT,
            );
            clip_round_rect(
                native_canvas,
                &inner_rect,
                &inner_round_rect,
                OH_Drawing_CanvasClipOp::DIFFERENCE,
            );
            canvas::OH_Drawing_CanvasAttachPen(native_canvas.as_ptr(), native_pen.0.as_ptr());
            canvas::OH_Drawing_CanvasDrawRoundRect(
                native_canvas.as_ptr(),
                center_round_rect.0.as_ptr(),
            );
            canvas::OH_Drawing_CanvasDetachPen(native_canvas.as_ptr());
            canvas::OH_Drawing_CanvasRestore(native_canvas.as_ptr());
        }
        Ok(())
    }

    fn draw_shadow(
        &self,
        native_canvas: NonNull<OH_Drawing_Canvas>,
        shadow: &Shadow,
    ) -> Result<()> {
        let content_mask = OwnedRect::from_bounds(shadow.content_mask.bounds)?;
        let shadow_rect = OwnedRect::from_bounds(shadow.bounds)?;
        let shadow_round_rect = OwnedRoundRect::new(&shadow_rect, shadow.corner_radii)?;
        let element_rect = OwnedRect::from_bounds(shadow.element_bounds)?;
        let element_round_rect = OwnedRoundRect::new(&element_rect, shadow.element_corner_radii)?;

        if shadow.inset == 0 {
            let native_brush = OwnedBrush::new_blurred(
                argb(shadow.color.to_rgb()),
                shadow.blur_radius.0,
                OH_Drawing_BlendMode::BLEND_MODE_SRC_OVER,
            )?;
            // SAFETY: All handles remain live through the balanced clip stack.
            unsafe {
                canvas::OH_Drawing_CanvasSave(native_canvas.as_ptr());
                clip_rect(native_canvas, &content_mask);
                draw_round_rect(
                    native_canvas,
                    &shadow_rect,
                    &shadow_round_rect,
                    &native_brush,
                );
                canvas::OH_Drawing_CanvasRestore(native_canvas.as_ptr());
            }
            return Ok(());
        }

        let fill_brush = OwnedBrush::new(argb(shadow.color.to_rgb()))?;
        let erase_brush = OwnedBrush::new_blurred(
            0xFFFFFFFF,
            shadow.blur_radius.0,
            OH_Drawing_BlendMode::BLEND_MODE_DST_OUT,
        )?;

        // The GPUI inset-shadow shader paints the element, then subtracts a
        // blurred rounded "hole". A Native Drawing layer with DST_OUT has the
        // same alpha composition and preserves the content beneath it.
        // SAFETY: The saved layer and clip stack are balanced, and all handles
        // remain live until both restores complete.
        unsafe {
            canvas::OH_Drawing_CanvasSave(native_canvas.as_ptr());
            clip_rect(native_canvas, &content_mask);
            clip_round_rect(
                native_canvas,
                &element_rect,
                &element_round_rect,
                OH_Drawing_CanvasClipOp::INTERSECT,
            );
            canvas::OH_Drawing_CanvasSaveLayer(
                native_canvas.as_ptr(),
                element_rect.0.as_ptr(),
                std::ptr::null(),
            );
            draw_round_rect(
                native_canvas,
                &element_rect,
                &element_round_rect,
                &fill_brush,
            );
            draw_round_rect(
                native_canvas,
                &shadow_rect,
                &shadow_round_rect,
                &erase_brush,
            );
            canvas::OH_Drawing_CanvasRestore(native_canvas.as_ptr());
            canvas::OH_Drawing_CanvasRestore(native_canvas.as_ptr());
        }
        Ok(())
    }

    fn draw_path(
        &self,
        native_canvas: NonNull<OH_Drawing_Canvas>,
        scene_path: &Path<ScaledPixels>,
    ) -> Result<()> {
        let native_path = OwnedPath::from_gpui(scene_path)?;
        let content_mask = OwnedRect::from_bounds(scene_path.content_mask.bounds)?;

        match scene_path.color.kind() {
            BackgroundKind::Solid(color) => {
                let native_brush = OwnedBrush::new(argb(color.to_rgb()))?;
                draw_brush_in_path(native_canvas, &content_mask, &native_path, &native_brush);
            }
            BackgroundKind::LinearGradient {
                angle,
                color_space,
                colors,
            } => {
                let native_brush =
                    OwnedBrush::linear_gradient(scene_path.bounds, angle, color_space, colors)?;
                draw_brush_in_path(native_canvas, &content_mask, &native_path, &native_brush);
            }
            BackgroundKind::PatternSlash {
                color,
                width,
                interval,
            } if width > 0.0 => {
                let diagonal_scale = std::f32::consts::FRAC_1_SQRT_2;
                let pattern = OwnedPath::slash_pattern(
                    scene_path.bounds,
                    (width + interval.max(0.0)) * diagonal_scale,
                )?;
                let native_pen = OwnedPen::new(argb(color.to_rgb()), width * diagonal_scale, None)?;
                // SAFETY: All native handles remain live through the balanced draw.
                unsafe {
                    canvas::OH_Drawing_CanvasSave(native_canvas.as_ptr());
                    clip_rect(native_canvas, &content_mask);
                    canvas::OH_Drawing_CanvasClipPath(
                        native_canvas.as_ptr(),
                        native_path.0.as_ptr(),
                        OH_Drawing_CanvasClipOp::INTERSECT,
                        true,
                    );
                    canvas::OH_Drawing_CanvasAttachPen(
                        native_canvas.as_ptr(),
                        native_pen.0.as_ptr(),
                    );
                    canvas::OH_Drawing_CanvasDrawPath(native_canvas.as_ptr(), pattern.0.as_ptr());
                    canvas::OH_Drawing_CanvasDetachPen(native_canvas.as_ptr());
                    canvas::OH_Drawing_CanvasRestore(native_canvas.as_ptr());
                }
            }
            BackgroundKind::Checkerboard { color, size } if size > 0.0 => {
                let pattern = OwnedPath::checkerboard(scene_path.bounds, size)?;
                let native_brush = OwnedBrush::new(argb(color.to_rgb()))?;
                // SAFETY: All native handles remain live through the balanced draw.
                unsafe {
                    canvas::OH_Drawing_CanvasSave(native_canvas.as_ptr());
                    clip_rect(native_canvas, &content_mask);
                    canvas::OH_Drawing_CanvasClipPath(
                        native_canvas.as_ptr(),
                        native_path.0.as_ptr(),
                        OH_Drawing_CanvasClipOp::INTERSECT,
                        true,
                    );
                    canvas::OH_Drawing_CanvasAttachBrush(
                        native_canvas.as_ptr(),
                        native_brush.native.as_ptr(),
                    );
                    canvas::OH_Drawing_CanvasDrawPath(native_canvas.as_ptr(), pattern.0.as_ptr());
                    canvas::OH_Drawing_CanvasDetachBrush(native_canvas.as_ptr());
                    canvas::OH_Drawing_CanvasRestore(native_canvas.as_ptr());
                }
            }
            BackgroundKind::PatternSlash { .. } | BackgroundKind::Checkerboard { .. } => {}
        }
        Ok(())
    }

    fn draw_underline(
        &self,
        native_canvas: NonNull<OH_Drawing_Canvas>,
        underline: &Underline,
    ) -> Result<()> {
        let thickness = underline.thickness.0.max(1.0);
        let native_pen = OwnedPen::new(argb(underline.color.to_rgb()), thickness, None)?;
        let native_path = if underline.wavy == gpui::PaddedBool32::from(true) {
            OwnedPath::wavy_underline(underline, thickness)?
        } else {
            OwnedPath::straight_underline(underline)?
        };
        let content_mask = OwnedRect::from_bounds(underline.content_mask.bounds)?;

        // SAFETY: All handles remain live through the balanced canvas stack.
        unsafe {
            canvas::OH_Drawing_CanvasSave(native_canvas.as_ptr());
            clip_rect(native_canvas, &content_mask);
            canvas::OH_Drawing_CanvasAttachPen(native_canvas.as_ptr(), native_pen.0.as_ptr());
            canvas::OH_Drawing_CanvasDrawPath(native_canvas.as_ptr(), native_path.0.as_ptr());
            canvas::OH_Drawing_CanvasDetachPen(native_canvas.as_ptr());
            canvas::OH_Drawing_CanvasRestore(native_canvas.as_ptr());
        }
        Ok(())
    }

    fn draw_monochrome_sprite(
        &self,
        native_canvas: NonNull<OH_Drawing_Canvas>,
        sprite: &MonochromeSprite,
        atlas: &OhosAtlas,
    ) -> Result<()> {
        let tile = atlas.tile_pixels(sprite.tile)?;
        if tile.kind != AtlasTextureKind::Monochrome {
            bail!("monochrome sprite references a non-monochrome atlas tile");
        }
        let color = sprite.color.to_rgb();
        let key = SpriteBitmapKey {
            texture_index: sprite.tile.texture_id.index,
            tile_id: sprite.tile.tile_id.0,
            revision: tile.revision,
            style: SpriteBitmapStyle::Monochrome {
                red: color.r.to_bits(),
                green: color.g.to_bits(),
                blue: color.b.to_bits(),
                alpha: color.a.to_bits(),
            },
        };
        self.draw_bitmap(
            native_canvas,
            &tile,
            key,
            || {
                let mut pixels = Vec::with_capacity(tile.bytes.len() * 4);
                for coverage in tile.bytes.iter().copied() {
                    let alpha = coverage as f32 / 255.0 * color.a;
                    pixels.push((color.r * alpha * 255.0).round() as u8);
                    pixels.push((color.g * alpha * 255.0).round() as u8);
                    pixels.push((color.b * alpha * 255.0).round() as u8);
                    pixels.push((alpha * 255.0).round() as u8);
                }
                Arc::from(pixels)
            },
            sprite.bounds,
            sprite.content_mask.bounds,
            None,
            Some(sprite.transformation),
            OH_Drawing_ColorFormat::COLOR_FORMAT_RGBA_8888,
        )
    }

    fn draw_subpixel_sprite(
        &self,
        native_canvas: NonNull<OH_Drawing_Canvas>,
        sprite: &SubpixelSprite,
        atlas: &OhosAtlas,
    ) -> Result<()> {
        let tile = atlas.tile_pixels(sprite.tile)?;
        if tile.kind != AtlasTextureKind::Subpixel {
            bail!("subpixel sprite references a non-subpixel atlas tile");
        }
        let color = sprite.color.to_rgb();
        let key = SpriteBitmapKey {
            texture_index: sprite.tile.texture_id.index,
            tile_id: sprite.tile.tile_id.0,
            revision: tile.revision,
            style: SpriteBitmapStyle::Subpixel {
                red: color.r.to_bits(),
                green: color.g.to_bits(),
                blue: color.b.to_bits(),
                alpha: color.a.to_bits(),
            },
        };
        self.draw_bitmap(
            native_canvas,
            &tile,
            key,
            || {
                let mut pixels = Vec::with_capacity(tile.bytes.len());
                for coverage in tile.bytes.chunks_exact(4) {
                    let alpha = coverage[3] as f32 / 255.0 * color.a;
                    pixels.push((color.r * coverage[2] as f32 * color.a).round() as u8);
                    pixels.push((color.g * coverage[1] as f32 * color.a).round() as u8);
                    pixels.push((color.b * coverage[0] as f32 * color.a).round() as u8);
                    pixels.push((alpha * 255.0).round() as u8);
                }
                Arc::from(pixels)
            },
            sprite.bounds,
            sprite.content_mask.bounds,
            None,
            Some(sprite.transformation),
            OH_Drawing_ColorFormat::COLOR_FORMAT_RGBA_8888,
        )
    }

    fn draw_polychrome_sprite(
        &self,
        native_canvas: NonNull<OH_Drawing_Canvas>,
        sprite: &PolychromeSprite,
        atlas: &OhosAtlas,
    ) -> Result<()> {
        let tile = atlas.tile_pixels(sprite.tile)?;
        if tile.kind != AtlasTextureKind::Polychrome {
            bail!("polychrome sprite references a non-polychrome atlas tile");
        }
        let grayscale = sprite.grayscale == gpui::PaddedBool32::from(true);
        let key = SpriteBitmapKey {
            texture_index: sprite.tile.texture_id.index,
            tile_id: sprite.tile.tile_id.0,
            revision: tile.revision,
            style: SpriteBitmapStyle::Polychrome {
                grayscale,
                opacity: sprite.opacity.to_bits(),
            },
        };
        self.draw_bitmap(
            native_canvas,
            &tile,
            key,
            || {
                if !grayscale && sprite.opacity >= 1.0 {
                    return tile.bytes.clone();
                }
                let mut pixels = tile.bytes.to_vec();
                for pixel in pixels.chunks_exact_mut(4) {
                    if grayscale {
                        let luminance = (0.0722 * pixel[0] as f32
                            + 0.7152 * pixel[1] as f32
                            + 0.2126 * pixel[2] as f32)
                            .round() as u8;
                        pixel[0] = luminance;
                        pixel[1] = luminance;
                        pixel[2] = luminance;
                    }
                    if sprite.opacity < 1.0 {
                        let opacity = sprite.opacity.clamp(0.0, 1.0);
                        for channel in pixel {
                            *channel = (*channel as f32 * opacity).round() as u8;
                        }
                    }
                }
                Arc::from(pixels)
            },
            sprite.bounds,
            sprite.content_mask.bounds,
            Some(sprite.corner_radii),
            None,
            OH_Drawing_ColorFormat::COLOR_FORMAT_BGRA_8888,
        )
    }

    fn draw_bitmap(
        &self,
        native_canvas: NonNull<OH_Drawing_Canvas>,
        tile: &TilePixels,
        key: SpriteBitmapKey,
        build_pixels: impl FnOnce() -> Arc<[u8]>,
        destination: Bounds<gpui::ScaledPixels>,
        content_mask: Bounds<gpui::ScaledPixels>,
        corner_radii: Option<Corners<ScaledPixels>>,
        transformation: Option<gpui::TransformationMatrix>,
        color_format: OH_Drawing_ColorFormat,
    ) -> Result<()> {
        let native_bitmap = if let Some(native_bitmap) = {
            let mut bitmaps = self.sprite_bitmaps.borrow_mut();
            bitmaps.get_mut(&key).map(|cached| {
                cached.last_used_frame = self.frame_id;
                cached.bitmap.native
            })
        } {
            native_bitmap
        } else {
            let bitmap = OwnedBitmap::new(tile.size, build_pixels(), color_format)?;
            let native_bitmap = bitmap.native;
            self.sprite_bitmaps.borrow_mut().insert(
                key,
                CachedBitmap {
                    bitmap,
                    last_used_frame: self.frame_id,
                },
            );
            native_bitmap
        };
        let destination = OwnedRect::new(
            destination.origin.x.0,
            destination.origin.y.0,
            destination.origin.x.0 + destination.size.width.0,
            destination.origin.y.0 + destination.size.height.0,
        )?;
        let clip = OwnedRect::new(
            content_mask.origin.x.0,
            content_mask.origin.y.0,
            content_mask.origin.x.0 + content_mask.size.width.0,
            content_mask.origin.y.0 + content_mask.size.height.0,
        )?;
        let destination_round_rect = corner_radii
            .map(|radii| OwnedRoundRect::new(&destination, radii))
            .transpose()?;
        let transformation = transformation.map(OwnedMatrix::new).transpose()?;
        // SAFETY: All native objects remain live through this balanced
        // save/clip/draw/restore sequence.
        unsafe {
            canvas::OH_Drawing_CanvasSave(native_canvas.as_ptr());
            canvas::OH_Drawing_CanvasClipRect(
                native_canvas.as_ptr(),
                clip.0.as_ptr(),
                OH_Drawing_CanvasClipOp::INTERSECT,
                false,
            );
            if let Some(transformation) = transformation.as_ref() {
                canvas::OH_Drawing_CanvasConcatMatrix(
                    native_canvas.as_ptr(),
                    transformation.0.as_ptr(),
                );
            }
            if let Some(destination_round_rect) = destination_round_rect.as_ref() {
                canvas::OH_Drawing_CanvasClipRoundRect(
                    native_canvas.as_ptr(),
                    destination_round_rect.0.as_ptr(),
                    OH_Drawing_CanvasClipOp::INTERSECT,
                    true,
                );
            }
            canvas::OH_Drawing_CanvasDrawBitmapRect(
                native_canvas.as_ptr(),
                native_bitmap.as_ptr(),
                std::ptr::null(),
                destination.0.as_ptr(),
                self.sampling.0.as_ptr(),
            );
            canvas::OH_Drawing_CanvasRestore(native_canvas.as_ptr());
        }
        Ok(())
    }
}

fn configure_native_window(
    window: NonNull<c_void>,
    width: u32,
    height: u32,
) -> Result<OH_Drawing_Image_Info> {
    let width = i32::try_from(width).context("XComponent width does not fit in i32")?;
    let height = i32::try_from(height).context("XComponent height does not fit in i32")?;
    if width == 0 || height == 0 {
        bail!("cannot configure a Native Drawing surface with zero size");
    }

    // SAFETY: The XComponent callback supplies a live OHNativeWindow and the
    // variadic arguments match SET_BUFFER_GEOMETRY's two i32 parameters.
    let geometry_status = unsafe {
        OH_NativeWindow_NativeWindowHandleOpt(
            window.as_ptr().cast(),
            NativeWindowOperation::SET_BUFFER_GEOMETRY as i32,
            width,
            height,
        )
    };
    if geometry_status != 0 {
        bail!("setting NativeWindow buffer geometry failed with status {geometry_status}");
    }

    // Live desktop resize emits many intermediate dimensions. Scaling the
    // latest buffer avoids a frozen/blank surface until the next exact-size
    // GPUI frame is ready.
    // SAFETY: The window is live and this scaling value is available in API 12+.
    let scaling_status = unsafe {
        OH_NativeWindow_NativeWindowSetScalingModeV2(
            window.as_ptr().cast(),
            OHScalingModeV2::OH_SCALING_MODE_SCALE_TO_WINDOW_V2,
        )
    };
    if scaling_status != 0 {
        bail!("setting NativeWindow scaling mode failed with status {scaling_status}");
    }

    Ok(OH_Drawing_Image_Info {
        width,
        height,
        colorType: OH_Drawing_ColorFormat::COLOR_FORMAT_RGBA_8888,
        alphaType: OH_Drawing_AlphaFormat::ALPHA_FORMAT_PREMUL,
    })
}

fn create_gpu_surface(
    window: NonNull<c_void>,
    image_info: OH_Drawing_Image_Info,
) -> Result<(NonNull<OH_Drawing_GpuContext>, NonNull<OH_Drawing_Surface>)> {
    // SAFETY: This creates a new, independently owned Native Drawing GPU
    // context. Its owner destroys it after the associated surface.
    let gpu_context = NonNull::new(unsafe { gpu_context::OH_Drawing_GpuContextCreate() })
        .ok_or_else(|| anyhow!("OH_Drawing_GpuContextCreate returned null"))?;
    // SAFETY: `gpu_context` and `window` are live for this call and
    // `image_info` describes the XComponent surface dimensions.
    let native_surface = unsafe {
        surface::OH_Drawing_SurfaceCreateOnScreen(gpu_context.as_ptr(), image_info, window.as_ptr())
    };
    let Some(native_surface) = NonNull::new(native_surface) else {
        // SAFETY: The context was created above and no surface retained it.
        unsafe { gpu_context::OH_Drawing_GpuContextDestroy(gpu_context.as_ptr()) };
        bail!("OH_Drawing_SurfaceCreateOnScreen returned null");
    };
    track_native_object(NativeObjectKind::GpuContext, ());
    track_native_object(NativeObjectKind::Surface, ());
    Ok((gpu_context, native_surface))
}

impl Drop for NativeDrawingSurface {
    fn drop(&mut self) {
        self.destroy_gpu_surface();
    }
}

struct OwnedBitmap {
    native: NonNull<OH_Drawing_Bitmap>,
    _pixels: Arc<[u8]>,
}

impl OwnedBitmap {
    fn new(
        size: gpui::Size<gpui::DevicePixels>,
        pixels: Arc<[u8]>,
        color_format: OH_Drawing_ColorFormat,
    ) -> Result<Self> {
        let width = u32::try_from(size.width.0).context("negative bitmap width")?;
        let height = u32::try_from(size.height.0).context("negative bitmap height")?;
        let mut image_info = OH_Drawing_Image_Info {
            width: i32::try_from(width)?,
            height: i32::try_from(height)?,
            colorType: color_format,
            alphaType: OH_Drawing_AlphaFormat::ALPHA_FORMAT_PREMUL,
        };
        let row_bytes = width.checked_mul(4).context("bitmap row size overflowed")?;
        // SAFETY: Native Drawing only reads this pointer. The pixel buffer is
        // correctly sized, stays allocated inside this owner, and outlives the
        // native bitmap.
        let native = NonNull::new(unsafe {
            bitmap::OH_Drawing_BitmapCreateFromPixels(
                &mut image_info,
                pixels.as_ptr().cast_mut().cast(),
                row_bytes,
            )
        })
        .ok_or_else(|| anyhow!("OH_Drawing_BitmapCreateFromPixels returned null"))?;
        Ok(track_native_object(
            NativeObjectKind::Bitmap,
            Self {
                native,
                _pixels: pixels,
            },
        ))
    }
}

impl Drop for OwnedBitmap {
    fn drop(&mut self) {
        // SAFETY: The bitmap is uniquely owned by this value.
        unsafe { bitmap::OH_Drawing_BitmapDestroy(self.native.as_ptr()) };
        release_native_object(NativeObjectKind::Bitmap);
    }
}

struct OwnedSamplingOptions(NonNull<OH_Drawing_SamplingOptions>);

impl OwnedSamplingOptions {
    fn new() -> Result<Self> {
        // SAFETY: Creates a new independently owned sampling object.
        NonNull::new(unsafe {
            sampling_options::OH_Drawing_SamplingOptionsCreate(
                OH_Drawing_FilterMode::FILTER_MODE_LINEAR,
                OH_Drawing_MipmapMode::MIPMAP_MODE_NONE,
            )
        })
        .map(|native| track_native_object(NativeObjectKind::Sampling, Self(native)))
        .ok_or_else(|| anyhow!("OH_Drawing_SamplingOptionsCreate returned null"))
    }
}

impl Drop for OwnedSamplingOptions {
    fn drop(&mut self) {
        // SAFETY: The sampling options are uniquely owned by this value.
        unsafe { sampling_options::OH_Drawing_SamplingOptionsDestroy(self.0.as_ptr()) };
        release_native_object(NativeObjectKind::Sampling);
    }
}

struct OwnedBlur {
    filter: NonNull<OH_Drawing_Filter>,
    mask: NonNull<OH_Drawing_MaskFilter>,
}

impl OwnedBlur {
    fn new(sigma: f32) -> Result<Self> {
        // SAFETY: Both calls create independently owned native objects. The
        // filter retains the mask for drawing while this owner keeps both live.
        let mask = NonNull::new(unsafe {
            mask_filter::OH_Drawing_MaskFilterCreateBlur(
                mask_filter::OH_Drawing_BlurType::NORMAL,
                sigma,
                true,
            )
        })
        .ok_or_else(|| anyhow!("OH_Drawing_MaskFilterCreateBlur returned null"))?;
        let Some(filter) = NonNull::new(unsafe { filter::OH_Drawing_FilterCreate() }) else {
            // SAFETY: `mask` was created above and has not been transferred.
            unsafe { mask_filter::OH_Drawing_MaskFilterDestroy(mask.as_ptr()) };
            bail!("OH_Drawing_FilterCreate returned null");
        };
        // SAFETY: Both objects are live and remain owned by this value.
        unsafe { filter::OH_Drawing_FilterSetMaskFilter(filter.as_ptr(), mask.as_ptr()) };
        track_native_object(NativeObjectKind::Filter, ());
        track_native_object(NativeObjectKind::MaskFilter, ());
        Ok(Self { filter, mask })
    }
}

impl Drop for OwnedBlur {
    fn drop(&mut self) {
        // SAFETY: The filter is destroyed before the mask it references.
        unsafe {
            filter::OH_Drawing_FilterDestroy(self.filter.as_ptr());
            mask_filter::OH_Drawing_MaskFilterDestroy(self.mask.as_ptr());
        }
        release_native_object(NativeObjectKind::Filter);
        release_native_object(NativeObjectKind::MaskFilter);
    }
}

struct OwnedBrush {
    native: NonNull<OH_Drawing_Brush>,
    _blur: Option<OwnedBlur>,
    _shader: Option<OwnedShaderEffect>,
}

impl OwnedBrush {
    fn new(color: u32) -> Result<Self> {
        // SAFETY: Creates a new brush released by `Drop`.
        let brush = NonNull::new(unsafe { brush::OH_Drawing_BrushCreate() })
            .ok_or_else(|| anyhow!("OH_Drawing_BrushCreate returned null"))?;
        // SAFETY: The brush is live and uniquely owned.
        unsafe {
            brush::OH_Drawing_BrushSetAntiAlias(brush.as_ptr(), true);
            brush::OH_Drawing_BrushSetColor(brush.as_ptr(), color);
        }
        Ok(track_native_object(
            NativeObjectKind::Brush,
            Self {
                native: brush,
                _blur: None,
                _shader: None,
            },
        ))
    }

    fn new_blurred(color: u32, sigma: f32, blend_mode: OH_Drawing_BlendMode) -> Result<Self> {
        let mut native_brush = Self::new(color)?;
        let blur = if sigma > 0.0 {
            Some(OwnedBlur::new(sigma)?)
        } else {
            None
        };
        // SAFETY: The brush is live, and any filter remains owned by the brush
        // wrapper until after the draw and native brush destruction.
        unsafe {
            brush::OH_Drawing_BrushSetBlendMode(native_brush.native.as_ptr(), blend_mode);
            if let Some(blur) = blur.as_ref() {
                brush::OH_Drawing_BrushSetFilter(
                    native_brush.native.as_ptr(),
                    blur.filter.as_ptr(),
                );
            }
        }
        native_brush._blur = blur;
        Ok(native_brush)
    }

    fn linear_gradient(
        bounds: Bounds<ScaledPixels>,
        angle: f32,
        color_space: ColorSpace,
        stops: [LinearColorStop; 2],
    ) -> Result<Self> {
        let mut native_brush = Self::new(0xFFFFFFFF)?;
        let shader = OwnedShaderEffect::linear_gradient(bounds, angle, color_space, stops)?;
        // SAFETY: Both handles remain live together in this wrapper until the
        // native brush has been destroyed.
        unsafe {
            brush::OH_Drawing_BrushSetShaderEffect(
                native_brush.native.as_ptr(),
                shader.native.as_ptr(),
            )
        };
        native_brush._shader = Some(shader);
        Ok(native_brush)
    }
}

impl Drop for OwnedBrush {
    fn drop(&mut self) {
        // SAFETY: The brush is uniquely owned by this value.
        unsafe { brush::OH_Drawing_BrushDestroy(self.native.as_ptr()) };
        release_native_object(NativeObjectKind::Brush);
    }
}

struct OwnedPoint(NonNull<OH_Drawing_Point>);

impl OwnedPoint {
    fn new(x: f32, y: f32) -> Result<Self> {
        // SAFETY: Creates a new, independently owned point.
        NonNull::new(unsafe { point::OH_Drawing_PointCreate(x, y) })
            .map(|native| track_native_object(NativeObjectKind::Point, Self(native)))
            .ok_or_else(|| anyhow!("OH_Drawing_PointCreate returned null"))
    }
}

impl Drop for OwnedPoint {
    fn drop(&mut self) {
        // SAFETY: The point is uniquely owned by this value.
        unsafe { point::OH_Drawing_PointDestroy(self.0.as_ptr()) };
        release_native_object(NativeObjectKind::Point);
    }
}

struct OwnedMatrix(NonNull<OH_Drawing_Matrix>);

impl OwnedMatrix {
    fn new(transformation: gpui::TransformationMatrix) -> Result<Self> {
        // SAFETY: Creates a new, independently owned matrix.
        let native = NonNull::new(unsafe { matrix::OH_Drawing_MatrixCreate() })
            .ok_or_else(|| anyhow!("OH_Drawing_MatrixCreate returned null"))?;
        // SAFETY: `native` is live. GPUI stores this affine matrix row-major;
        // Native Drawing accepts the corresponding scale/skew/translation rows.
        unsafe {
            matrix::OH_Drawing_MatrixSetMatrix(
                native.as_ptr(),
                transformation.rotation_scale[0][0],
                transformation.rotation_scale[0][1],
                transformation.translation[0],
                transformation.rotation_scale[1][0],
                transformation.rotation_scale[1][1],
                transformation.translation[1],
                0.0,
                0.0,
                1.0,
            )
        };
        Ok(track_native_object(NativeObjectKind::Matrix, Self(native)))
    }
}

impl Drop for OwnedMatrix {
    fn drop(&mut self) {
        // SAFETY: The matrix is uniquely owned by this value.
        unsafe { matrix::OH_Drawing_MatrixDestroy(self.0.as_ptr()) };
        release_native_object(NativeObjectKind::Matrix);
    }
}

struct OwnedShaderEffect {
    native: NonNull<OH_Drawing_ShaderEffect>,
    _start: OwnedPoint,
    _end: OwnedPoint,
    _colors: Vec<u32>,
    _positions: Vec<f32>,
}

impl OwnedShaderEffect {
    fn linear_gradient(
        bounds: Bounds<ScaledPixels>,
        angle: f32,
        color_space: ColorSpace,
        stops: [LinearColorStop; 2],
    ) -> Result<Self> {
        let width = bounds.size.width.0.max(f32::EPSILON);
        let height = bounds.size.height.0.max(f32::EPSILON);
        let radians = (angle.rem_euclid(360.0) - 90.0).to_radians();
        let mut direction = (radians.cos(), radians.sin());
        if width > height {
            direction.1 *= height / width;
        } else {
            direction.0 *= width / height;
        }
        let direction_length = direction.0.hypot(direction.1).max(f32::EPSILON);
        let dominant_size = if direction.0.abs() > direction.1.abs() {
            width
        } else {
            height
        };
        let gradient = (
            direction.0 / direction_length / dominant_size,
            direction.1 / direction_length / dominant_size,
        );
        let gradient_length_squared =
            (gradient.0 * gradient.0 + gradient.1 * gradient.1).max(f32::EPSILON);
        let span = (
            gradient.0 / gradient_length_squared,
            gradient.1 / gradient_length_squared,
        );
        let center = (
            bounds.origin.x.0 + width * 0.5,
            bounds.origin.y.0 + height * 0.5,
        );
        let start = OwnedPoint::new(center.0 - span.0 * 0.5, center.1 - span.1 * 0.5)?;
        let end = OwnedPoint::new(center.0 + span.0 * 0.5, center.1 + span.1 * 0.5)?;
        let (colors, positions) = gradient_samples(color_space, stops);
        let color_count = u32::try_from(colors.len()).context("gradient stop count exceeds u32")?;
        // SAFETY: Both points and backing arrays remain live in the returned
        // owner for at least as long as the native shader effect.
        let native = NonNull::new(unsafe {
            shader_effect::OH_Drawing_ShaderEffectCreateLinearGradient(
                start.0.as_ptr(),
                end.0.as_ptr(),
                colors.as_ptr(),
                positions.as_ptr(),
                color_count,
                shader_effect::OH_Drawing_TileMode::CLAMP,
            )
        })
        .ok_or_else(|| anyhow!("OH_Drawing_ShaderEffectCreateLinearGradient returned null"))?;
        Ok(track_native_object(
            NativeObjectKind::Shader,
            Self {
                native,
                _start: start,
                _end: end,
                _colors: colors,
                _positions: positions,
            },
        ))
    }
}

impl Drop for OwnedShaderEffect {
    fn drop(&mut self) {
        // SAFETY: The shader effect is uniquely owned by this value.
        unsafe { shader_effect::OH_Drawing_ShaderEffectDestroy(self.native.as_ptr()) };
        release_native_object(NativeObjectKind::Shader);
    }
}

struct OwnedRect(NonNull<OH_Drawing_Rect>);

impl OwnedRect {
    fn new(left: f32, top: f32, right: f32, bottom: f32) -> Result<Self> {
        // SAFETY: Creates a new rectangle released by `Drop`.
        NonNull::new(unsafe { rect::OH_Drawing_RectCreate(left, top, right, bottom) })
            .map(|native| track_native_object(NativeObjectKind::Rect, Self(native)))
            .ok_or_else(|| anyhow!("OH_Drawing_RectCreate returned null"))
    }

    fn from_bounds(bounds: Bounds<ScaledPixels>) -> Result<Self> {
        Self::new(
            bounds.origin.x.0,
            bounds.origin.y.0,
            bounds.origin.x.0 + bounds.size.width.0,
            bounds.origin.y.0 + bounds.size.height.0,
        )
    }
}

impl Drop for OwnedRect {
    fn drop(&mut self) {
        // SAFETY: The rectangle is uniquely owned by this value.
        unsafe { rect::OH_Drawing_RectDestroy(self.0.as_ptr()) };
        release_native_object(NativeObjectKind::Rect);
    }
}

struct OwnedRoundRect(NonNull<OH_Drawing_RoundRect>);

impl OwnedRoundRect {
    fn new(rect: &OwnedRect, radii: Corners<ScaledPixels>) -> Result<Self> {
        // SAFETY: The source rectangle is live for the call and the returned
        // round rectangle is independently owned.
        let native = NonNull::new(unsafe {
            round_rect::OH_Drawing_RoundRectCreate(rect.0.as_ptr(), 0.0, 0.0)
        })
        .ok_or_else(|| anyhow!("OH_Drawing_RoundRectCreate returned null"))?;
        // SAFETY: `native` is live and each API receives a plain pair of radii.
        unsafe {
            set_round_rect_corner(
                native,
                round_rect::OH_Drawing_CornerPos::CORNER_POS_TOP_LEFT,
                radii.top_left.0,
                radii.top_left.0,
            );
            set_round_rect_corner(
                native,
                round_rect::OH_Drawing_CornerPos::CORNER_POS_TOP_RIGHT,
                radii.top_right.0,
                radii.top_right.0,
            );
            set_round_rect_corner(
                native,
                round_rect::OH_Drawing_CornerPos::CORNER_POS_BOTTOM_RIGHT,
                radii.bottom_right.0,
                radii.bottom_right.0,
            );
            set_round_rect_corner(
                native,
                round_rect::OH_Drawing_CornerPos::CORNER_POS_BOTTOM_LEFT,
                radii.bottom_left.0,
                radii.bottom_left.0,
            );
        }
        Ok(track_native_object(
            NativeObjectKind::RoundRect,
            Self(native),
        ))
    }

    fn new_elliptical(rect: &OwnedRect, radii: [(f32, f32); 4]) -> Result<Self> {
        let native = NonNull::new(unsafe {
            round_rect::OH_Drawing_RoundRectCreate(rect.0.as_ptr(), 0.0, 0.0)
        })
        .ok_or_else(|| anyhow!("OH_Drawing_RoundRectCreate returned null"))?;
        let positions = [
            round_rect::OH_Drawing_CornerPos::CORNER_POS_TOP_LEFT,
            round_rect::OH_Drawing_CornerPos::CORNER_POS_TOP_RIGHT,
            round_rect::OH_Drawing_CornerPos::CORNER_POS_BOTTOM_RIGHT,
            round_rect::OH_Drawing_CornerPos::CORNER_POS_BOTTOM_LEFT,
        ];
        // SAFETY: `native` is live and both arrays have exactly four entries.
        unsafe {
            for (position, (x, y)) in positions.into_iter().zip(radii) {
                set_round_rect_corner(native, position, x, y);
            }
        }
        Ok(track_native_object(
            NativeObjectKind::RoundRect,
            Self(native),
        ))
    }
}

impl Drop for OwnedRoundRect {
    fn drop(&mut self) {
        // SAFETY: The round rectangle is uniquely owned by this value.
        unsafe { round_rect::OH_Drawing_RoundRectDestroy(self.0.as_ptr()) };
        release_native_object(NativeObjectKind::RoundRect);
    }
}

struct OwnedPen(NonNull<OH_Drawing_Pen>);

impl OwnedPen {
    fn new(color: u32, width: f32, path_effect: Option<&OwnedPathEffect>) -> Result<Self> {
        // SAFETY: Creates a new, independently owned pen.
        let native = NonNull::new(unsafe { pen::OH_Drawing_PenCreate() })
            .ok_or_else(|| anyhow!("OH_Drawing_PenCreate returned null"))?;
        // SAFETY: The pen is live and the optional path effect outlives every
        // draw using the configured pen.
        unsafe {
            pen::OH_Drawing_PenSetAntiAlias(native.as_ptr(), true);
            pen::OH_Drawing_PenSetColor(native.as_ptr(), color);
            pen::OH_Drawing_PenSetWidth(native.as_ptr(), width);
            pen::OH_Drawing_PenSetJoin(
                native.as_ptr(),
                pen::OH_Drawing_PenLineJoinStyle::LINE_ROUND_JOIN,
            );
            pen::OH_Drawing_PenSetCap(
                native.as_ptr(),
                pen::OH_Drawing_PenLineCapStyle::LINE_ROUND_CAP,
            );
            if let Some(path_effect) = path_effect {
                pen::OH_Drawing_PenSetPathEffect(native.as_ptr(), path_effect.native.as_ptr());
            }
        }
        Ok(track_native_object(NativeObjectKind::Pen, Self(native)))
    }
}

impl Drop for OwnedPen {
    fn drop(&mut self) {
        // SAFETY: The pen is uniquely owned by this value.
        unsafe { pen::OH_Drawing_PenDestroy(self.0.as_ptr()) };
        release_native_object(NativeObjectKind::Pen);
    }
}

struct OwnedPathEffect {
    native: NonNull<OH_Drawing_PathEffect>,
    _intervals: [f32; 2],
}

impl OwnedPathEffect {
    fn dash(mut intervals: [f32; 2]) -> Result<Self> {
        // SAFETY: Native Drawing copies the two interval values into the new
        // independently owned effect. We retain them as well for API safety.
        let native = NonNull::new(unsafe {
            path_effect::OH_Drawing_CreateDashPathEffect(intervals.as_mut_ptr(), 2, 0.0)
        })
        .ok_or_else(|| anyhow!("OH_Drawing_CreateDashPathEffect returned null"))?;
        Ok(track_native_object(
            NativeObjectKind::PathEffect,
            Self {
                native,
                _intervals: intervals,
            },
        ))
    }
}

impl Drop for OwnedPathEffect {
    fn drop(&mut self) {
        // SAFETY: The path effect is uniquely owned by this value.
        unsafe { path_effect::OH_Drawing_PathEffectDestroy(self.native.as_ptr()) };
        release_native_object(NativeObjectKind::PathEffect);
    }
}

struct OwnedPath(NonNull<OH_Drawing_Path>);

impl OwnedPath {
    fn new() -> Result<Self> {
        // SAFETY: Creates a new, independently owned path.
        NonNull::new(unsafe { path::OH_Drawing_PathCreate() })
            .map(|native| track_native_object(NativeObjectKind::Path, Self(native)))
            .ok_or_else(|| anyhow!("OH_Drawing_PathCreate returned null"))
    }

    fn from_gpui(scene_path: &Path<ScaledPixels>) -> Result<Self> {
        let native_path = Self::new()?;
        let mut triangles = scene_path.vertices.chunks_exact(3);
        for triangle in &mut triangles {
            let [first, second, third] = triangle else {
                bail!("GPUI path yielded an incomplete triangle");
            };
            let curve_triangle = first.st_position.x == 0.0
                && first.st_position.y == 0.0
                && second.st_position.x == 0.5
                && second.st_position.y == 0.0
                && third.st_position.x == 1.0
                && third.st_position.y == 1.0;
            // SAFETY: The native path is live. GPUI stores both regular
            // tessellation triangles and quadratic curve wedges in triplets.
            unsafe {
                path::OH_Drawing_PathMoveTo(
                    native_path.0.as_ptr(),
                    first.xy_position.x.0,
                    first.xy_position.y.0,
                );
                if curve_triangle {
                    path::OH_Drawing_PathQuadTo(
                        native_path.0.as_ptr(),
                        second.xy_position.x.0,
                        second.xy_position.y.0,
                        third.xy_position.x.0,
                        third.xy_position.y.0,
                    );
                } else {
                    path::OH_Drawing_PathLineTo(
                        native_path.0.as_ptr(),
                        second.xy_position.x.0,
                        second.xy_position.y.0,
                    );
                    path::OH_Drawing_PathLineTo(
                        native_path.0.as_ptr(),
                        third.xy_position.x.0,
                        third.xy_position.y.0,
                    );
                }
                path::OH_Drawing_PathClose(native_path.0.as_ptr());
            }
        }
        if !triangles.remainder().is_empty() {
            bail!("GPUI path vertex count is not divisible by three");
        }
        Ok(native_path)
    }

    fn straight_underline(underline: &Underline) -> Result<Self> {
        let native_path = Self::new()?;
        let left = underline.bounds.origin.x.0;
        let right = left + underline.bounds.size.width.0;
        let center_y = underline.bounds.origin.y.0 + underline.bounds.size.height.0 * 0.5;
        // SAFETY: The native path is live for both mutations.
        unsafe {
            path::OH_Drawing_PathMoveTo(native_path.0.as_ptr(), left, center_y);
            path::OH_Drawing_PathLineTo(native_path.0.as_ptr(), right, center_y);
        }
        Ok(native_path)
    }

    fn wavy_underline(underline: &Underline, thickness: f32) -> Result<Self> {
        let native_path = Self::new()?;
        let left = underline.bounds.origin.x.0;
        let right = left + underline.bounds.size.width.0;
        let height = underline.bounds.size.height.0.max(thickness);
        let center_y = underline.bounds.origin.y.0 + underline.bounds.size.height.0 * 0.5;
        let amplitude = thickness * 0.8;
        let phase_per_x = std::f32::consts::TAU * thickness / (height * height);
        let period = std::f32::consts::TAU / phase_per_x;
        let step = (period / 12.0).clamp(0.5, 4.0);

        // SAFETY: The native path is live for all sampled line segments.
        unsafe {
            path::OH_Drawing_PathMoveTo(native_path.0.as_ptr(), left, center_y);
            let mut x = left + step;
            while x < right {
                let y = center_y + ((x - left) * phase_per_x).sin() * amplitude;
                path::OH_Drawing_PathLineTo(native_path.0.as_ptr(), x, y);
                x += step;
            }
            let final_y = center_y + ((right - left) * phase_per_x).sin() * amplitude;
            path::OH_Drawing_PathLineTo(native_path.0.as_ptr(), right, final_y);
        }
        Ok(native_path)
    }

    fn slash_pattern(bounds: Bounds<ScaledPixels>, spacing: f32) -> Result<Self> {
        let native_path = Self::new()?;
        let spacing = spacing.max(0.25);
        let center_x = bounds.origin.x.0 + bounds.size.width.0 * 0.5;
        let center_y = bounds.origin.y.0 + bounds.size.height.0 * 0.5;
        let extent = bounds.size.width.0.hypot(bounds.size.height.0).max(spacing);
        let inverse_square_root_two = std::f32::consts::FRAC_1_SQRT_2;
        let direction = (inverse_square_root_two, -inverse_square_root_two);
        let normal = (inverse_square_root_two, inverse_square_root_two);
        let mut offset = -extent;
        // SAFETY: The path is live for all generated line segments.
        unsafe {
            while offset <= extent {
                let line_center = (center_x + normal.0 * offset, center_y + normal.1 * offset);
                path::OH_Drawing_PathMoveTo(
                    native_path.0.as_ptr(),
                    line_center.0 - direction.0 * extent,
                    line_center.1 - direction.1 * extent,
                );
                path::OH_Drawing_PathLineTo(
                    native_path.0.as_ptr(),
                    line_center.0 + direction.0 * extent,
                    line_center.1 + direction.1 * extent,
                );
                offset += spacing;
            }
        }
        Ok(native_path)
    }

    fn checkerboard(bounds: Bounds<ScaledPixels>, square_size: f32) -> Result<Self> {
        let native_path = Self::new()?;
        let width = bounds.size.width.0.max(0.0);
        let height = bounds.size.height.0.max(0.0);
        let columns = (width / square_size).ceil() as u32;
        let rows = (height / square_size).ceil() as u32;
        // SAFETY: The native path is live for all rectangle subpaths.
        unsafe {
            for row in 0..rows {
                for column in 0..columns {
                    if (row + column) % 2 == 0 {
                        continue;
                    }
                    let left = bounds.origin.x.0 + column as f32 * square_size;
                    let top = bounds.origin.y.0 + row as f32 * square_size;
                    let right = (left + square_size).min(bounds.origin.x.0 + width);
                    let bottom = (top + square_size).min(bounds.origin.y.0 + height);
                    path::OH_Drawing_PathMoveTo(native_path.0.as_ptr(), left, top);
                    path::OH_Drawing_PathLineTo(native_path.0.as_ptr(), right, top);
                    path::OH_Drawing_PathLineTo(native_path.0.as_ptr(), right, bottom);
                    path::OH_Drawing_PathLineTo(native_path.0.as_ptr(), left, bottom);
                    path::OH_Drawing_PathClose(native_path.0.as_ptr());
                }
            }
        }
        Ok(native_path)
    }
}

impl Drop for OwnedPath {
    fn drop(&mut self) {
        // SAFETY: The path is uniquely owned by this value.
        unsafe { path::OH_Drawing_PathDestroy(self.0.as_ptr()) };
        release_native_object(NativeObjectKind::Path);
    }
}

fn border_is_visible(widths: Edges<ScaledPixels>) -> bool {
    widths.top.0 > 0.0 || widths.right.0 > 0.0 || widths.bottom.0 > 0.0 || widths.left.0 > 0.0
}

fn maximum_border_width(widths: Edges<ScaledPixels>) -> f32 {
    widths
        .top
        .0
        .max(widths.right.0)
        .max(widths.bottom.0)
        .max(widths.left.0)
}

fn inset_bounds(bounds: Bounds<ScaledPixels>, widths: Edges<ScaledPixels>) -> Bounds<ScaledPixels> {
    let width = (bounds.size.width.0 - widths.left.0 - widths.right.0).max(0.0);
    let height = (bounds.size.height.0 - widths.top.0 - widths.bottom.0).max(0.0);
    Bounds {
        origin: gpui::point(
            ScaledPixels(bounds.origin.x.0 + widths.left.0),
            ScaledPixels(bounds.origin.y.0 + widths.top.0),
        ),
        size: gpui::size(ScaledPixels(width), ScaledPixels(height)),
    }
}

fn inset_elliptical_radii(
    radii: Corners<ScaledPixels>,
    widths: Edges<ScaledPixels>,
) -> [(f32, f32); 4] {
    [
        (
            (radii.top_left.0 - widths.left.0).max(0.0),
            (radii.top_left.0 - widths.top.0).max(0.0),
        ),
        (
            (radii.top_right.0 - widths.right.0).max(0.0),
            (radii.top_right.0 - widths.top.0).max(0.0),
        ),
        (
            (radii.bottom_right.0 - widths.right.0).max(0.0),
            (radii.bottom_right.0 - widths.bottom.0).max(0.0),
        ),
        (
            (radii.bottom_left.0 - widths.left.0).max(0.0),
            (radii.bottom_left.0 - widths.bottom.0).max(0.0),
        ),
    ]
}

unsafe fn set_round_rect_corner(
    round_rect: NonNull<OH_Drawing_RoundRect>,
    position: round_rect::OH_Drawing_CornerPos,
    x: f32,
    y: f32,
) {
    // SAFETY: The caller guarantees the round rectangle is live.
    unsafe {
        round_rect::OH_Drawing_RoundRectSetCorner(
            round_rect.as_ptr(),
            position,
            OH_Drawing_Corner_Radii {
                x: x.max(0.0),
                y: y.max(0.0),
            },
        )
    };
}

unsafe fn clip_rect(native_canvas: NonNull<OH_Drawing_Canvas>, rect: &OwnedRect) {
    // SAFETY: The caller guarantees both handles are live.
    unsafe {
        canvas::OH_Drawing_CanvasClipRect(
            native_canvas.as_ptr(),
            rect.0.as_ptr(),
            OH_Drawing_CanvasClipOp::INTERSECT,
            false,
        )
    };
}

unsafe fn clip_round_rect(
    native_canvas: NonNull<OH_Drawing_Canvas>,
    _rect: &OwnedRect,
    round_rect: &OwnedRoundRect,
    operation: OH_Drawing_CanvasClipOp,
) {
    // SAFETY: The caller guarantees all handles are live.
    unsafe {
        canvas::OH_Drawing_CanvasClipRoundRect(
            native_canvas.as_ptr(),
            round_rect.0.as_ptr(),
            operation,
            true,
        )
    };
}

unsafe fn draw_round_rect(
    native_canvas: NonNull<OH_Drawing_Canvas>,
    _rect: &OwnedRect,
    round_rect: &OwnedRoundRect,
    native_brush: &OwnedBrush,
) {
    // SAFETY: The caller guarantees all handles are live through the draw.
    unsafe {
        canvas::OH_Drawing_CanvasAttachBrush(native_canvas.as_ptr(), native_brush.native.as_ptr());
        canvas::OH_Drawing_CanvasDrawRoundRect(native_canvas.as_ptr(), round_rect.0.as_ptr());
        canvas::OH_Drawing_CanvasDetachBrush(native_canvas.as_ptr());
    }
}

fn draw_brush_in_path(
    native_canvas: NonNull<OH_Drawing_Canvas>,
    content_mask: &OwnedRect,
    native_path: &OwnedPath,
    native_brush: &OwnedBrush,
) {
    // SAFETY: Every handle remains live through this balanced draw sequence.
    unsafe {
        canvas::OH_Drawing_CanvasSave(native_canvas.as_ptr());
        clip_rect(native_canvas, content_mask);
        canvas::OH_Drawing_CanvasAttachBrush(native_canvas.as_ptr(), native_brush.native.as_ptr());
        canvas::OH_Drawing_CanvasDrawPath(native_canvas.as_ptr(), native_path.0.as_ptr());
        canvas::OH_Drawing_CanvasDetachBrush(native_canvas.as_ptr());
        canvas::OH_Drawing_CanvasRestore(native_canvas.as_ptr());
    }
}

fn gradient_samples(color_space: ColorSpace, stops: [LinearColorStop; 2]) -> (Vec<u32>, Vec<f32>) {
    if color_space == ColorSpace::Srgb
        || (stops[1].percentage - stops[0].percentage).abs() <= f32::EPSILON
    {
        return (
            stops.iter().map(|stop| argb(stop.color.to_rgb())).collect(),
            stops.iter().map(|stop| stop.percentage).collect(),
        );
    }

    // Native Drawing interpolates shader stops in sRGB. A dense Oklab stop
    // table preserves GPUI's requested interpolation while retaining native
    // clipping and path rasterization. 257 samples bound each native sRGB
    // segment to less than one 8-bit color step for normal UI gradients.
    const SAMPLE_COUNT: usize = 257;
    let from = rgba_to_oklab(stops[0].color.to_rgb());
    let to = rgba_to_oklab(stops[1].color.to_rgb());
    let mut colors = Vec::with_capacity(SAMPLE_COUNT);
    let mut positions = Vec::with_capacity(SAMPLE_COUNT);
    for index in 0..SAMPLE_COUNT {
        let t = index as f32 / (SAMPLE_COUNT - 1) as f32;
        colors.push(argb(oklab_to_rgba([
            from[0] + (to[0] - from[0]) * t,
            from[1] + (to[1] - from[1]) * t,
            from[2] + (to[2] - from[2]) * t,
            from[3] + (to[3] - from[3]) * t,
        ])));
        positions.push(stops[0].percentage + (stops[1].percentage - stops[0].percentage) * t);
    }
    (colors, positions)
}

fn rgba_to_oklab(color: Rgba) -> [f32; 4] {
    let red = srgb_to_linear(color.r);
    let green = srgb_to_linear(color.g);
    let blue = srgb_to_linear(color.b);
    let l = (0.412_221_46 * red + 0.536_332_55 * green + 0.051_445_995 * blue).cbrt();
    let m = (0.211_903_5 * red + 0.680_699_5 * green + 0.107_396_96 * blue).cbrt();
    let s = (0.088_302_46 * red + 0.281_718_85 * green + 0.629_978_7 * blue).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
        color.a,
    ]
}

fn oklab_to_rgba(color: [f32; 4]) -> Rgba {
    let l = (color[0] + 0.396_337_78 * color[1] + 0.215_803_76 * color[2]).powi(3);
    let m = (color[0] - 0.105_561_346 * color[1] - 0.063_854_17 * color[2]).powi(3);
    let s = (color[0] - 0.089_484_18 * color[1] - 1.291_485_5 * color[2]).powi(3);
    Rgba {
        r: linear_to_srgb(4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s),
        g: linear_to_srgb(-1.268_438 * l + 2.609_757_4 * m - 0.341_319_4 * s),
        b: linear_to_srgb(-0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s),
        a: color[3],
    }
}

fn srgb_to_linear(component: f32) -> f32 {
    if component <= 0.04045 {
        component / 12.92
    } else {
        ((component + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(component: f32) -> f32 {
    if component <= 0.003_130_8 {
        component * 12.92
    } else {
        1.055 * component.powf(1.0 / 2.4) - 0.055
    }
}

fn argb(color: Rgba) -> u32 {
    let alpha = (color.a.clamp(0.0, 1.0) * 255.0).round() as u32;
    let red = (color.r.clamp(0.0, 1.0) * 255.0).round() as u32;
    let green = (color.g.clamp(0.0, 1.0) * 255.0).round() as u32;
    let blue = (color.b.clamp(0.0, 1.0) * 255.0).round() as u32;
    (alpha << 24) | (red << 16) | (green << 8) | blue
}
