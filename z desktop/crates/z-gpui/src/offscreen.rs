//! Headless rendering.
//!
//! Renders a scene to a texture with no window and no compositor involved, then
//! writes it out as a PNG. Two jobs:
//!
//! - **Visual QA evidence.** The plan requires a reviewable artifact for every
//!   UI task; this produces one deterministically instead of relying on someone
//!   screenshotting their own desktop.
//! - **CI.** A build machine with no display can still prove the shell renders,
//!   and can diff the result against a baseline.
//!
//! The path deliberately shares `Renderer` with the windowed path, so what is
//! captured here is what the user sees — not a second implementation that can
//! drift.

use crate::geometry::Rect;
use crate::renderer::{BackendInfo, FrameStats, Renderer};
use crate::scene::Scene;
use crate::text::TextSystem;
use std::path::Path;

/// A GPU context with no surface attached.
pub struct OffscreenRenderer {
    renderer: Renderer,
    text: TextSystem,
    scale: f32,
}

/// Result of a headless capture.
#[derive(Debug, Clone)]
pub struct Capture {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA8, straight (non-premultiplied) alpha.
    pub pixels: Vec<u8>,
    pub stats: FrameStats,
    pub backend: BackendInfo,
}

impl Capture {
    pub fn write_png(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let file = std::fs::File::create(path.as_ref())
            .map_err(|e| format!("could not create {}: {e}", path.as_ref().display()))?;
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| format!("png header: {e}"))?;
        writer.write_image_data(&self.pixels).map_err(|e| format!("png data: {e}"))?;
        Ok(())
    }

    /// Colour at a point, for assertions in visual tests.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = ((y * self.width + x) * 4) as usize;
        Some([self.pixels[i], self.pixels[i + 1], self.pixels[i + 2], self.pixels[i + 3]])
    }
}

impl OffscreenRenderer {
    /// Acquire a device without a surface.
    ///
    /// Falls back to a software adapter when no GPU is available, so this works
    /// on a headless build machine.
    pub fn new(scale: f32) -> Result<Self, String> {
        pollster::block_on(Self::new_async(scale))
    }

    async fn new_async(scale: f32) -> Result<Self, String> {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::PRIMARY;
        let instance = wgpu::Instance::new(descriptor);

        let adapter = match instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: None,
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await
        {
            Ok(adapter) => adapter,
            // No hardware adapter: try the software path rather than failing,
            // so CI without a GPU still produces an artifact.
            Err(_) => instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    compatible_surface: None,
                    force_fallback_adapter: true,
                    ..Default::default()
                })
                .await
                .map_err(|e| format!("no usable adapter, not even a fallback: {e}"))?,
        };

        let info = adapter.get_info();
        let backend = BackendInfo {
            backend: format!("{:?}", info.backend),
            adapter: info.name.clone(),
            device_type: format!("{:?}", info.device_type),
        };

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("zero.offscreen.device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults()
                    .using_resolution(adapter.limits()),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
                ..Default::default()
            })
            .await
            .map_err(|e| format!("could not acquire a device: {e}"))?;

        let text = TextSystem::new(scale);
        let atlas_size = text.atlas().width();
        // Rgba8UnormSrgb matches what the windowed path presents to, so colours
        // land identically in both.
        let renderer =
            Renderer::new(device, queue, wgpu::TextureFormat::Rgba8UnormSrgb, backend, atlas_size);

        Ok(Self { renderer, text, scale: scale.max(0.1) })
    }

    pub fn backend(&self) -> &BackendInfo {
        self.renderer.backend_info()
    }

    /// Render `scene` at `viewport` logical size and read the pixels back.
    pub fn capture(
        &mut self,
        scene: &Scene,
        viewport: Rect,
        clear: z_tokens::Rgba,
    ) -> Result<Capture, String> {
        let width = ((viewport.width * self.scale).round() as u32).max(1);
        let height = ((viewport.height * self.scale).round() as u32).max(1);

        let target = self.renderer.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("zero.offscreen.target"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());

        let stats =
            self.renderer.render(&view, scene, &mut self.text, viewport, clear, Some(viewport));

        // Buffer rows must be a multiple of 256 bytes, so the readback is padded
        // and the padding stripped after mapping.
        let unpadded = width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded = unpadded.div_ceil(align) * align;

        let readback = self.renderer.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("zero.offscreen.readback"),
            size: (padded * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder =
            self.renderer.device().create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("zero.offscreen.copy"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.renderer.queue().submit(Some(encoder.finish()));

        let slice = readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        // Block until the copy has actually executed; without the wait the
        // mapping callback may never be invoked and the capture hangs.
        self.renderer
            .device()
            .poll(wgpu::PollType::Wait { submission_index: None, timeout: None })
            .map_err(|e| format!("device poll failed: {e}"))?;
        receiver
            .recv()
            .map_err(|e| format!("readback never completed: {e}"))?
            .map_err(|e| format!("could not map the readback buffer: {e}"))?;

        let mapped = slice
            .get_mapped_range()
            .map_err(|e| format!("could not read the mapped range: {e}"))?;
        let mut pixels = Vec::with_capacity((unpadded * height) as usize);
        for row in 0..height {
            let start = (row * padded) as usize;
            pixels.extend_from_slice(&mapped[start..start + unpadded as usize]);
        }
        drop(mapped);
        readback.unmap();

        Ok(Capture { width, height, pixels, stats, backend: self.renderer.backend_info().clone() })
    }
}
