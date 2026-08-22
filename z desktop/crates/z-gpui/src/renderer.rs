//! ZeroRender — the GPU backend boundary.
//!
//! Everything above this module speaks in [`Scene`]s. This is the only place
//! that knows a graphics API exists, which is the property that keeps the
//! backend replaceable: no wgpu type appears in any public signature outside
//! this file.
//!
//! The backend in use (Direct3D 12, Metal or Vulkan) is chosen by capability at
//! runtime, never assumed at compile time.

use crate::geometry::Rect;
use crate::scene::Scene;
use crate::text::TextSystem;
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Globals {
    viewport: [f32; 2],
    _padding: [f32; 2],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
/// One instanced quad.
///
/// Field order must match the `vertex_attr_array!` declaration below, and there
/// must be no padding between fields: that macro computes each attribute's
/// offset by summing the sizes of the formats it is given, so a field the macro
/// does not know about silently shifts every attribute after it. The size
/// assertion in this module's tests is what keeps the two in step.
struct QuadInstance {
    rect: [f32; 4],
    background: [f32; 4],
    border_color: [f32; 4],
    clip: [f32; 4],
    params: [f32; 2],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct GlyphInstance {
    rect: [f32; 4],
    uv: [f32; 4],
    color: [f32; 4],
    clip: [f32; 4],
}

/// What the renderer did with a frame. Reported rather than inferred, so the
/// frame budget can be checked against real work.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameStats {
    pub quads: u32,
    pub glyphs: u32,
    pub draw_calls: u32,
    /// True when nothing changed and the GPU was not touched at all.
    pub skipped: bool,
}

/// Which backend actually ended up in use, and on which adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendInfo {
    pub backend: String,
    pub adapter: String,
    pub device_type: String,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_format: wgpu::TextureFormat,

    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,

    quad_pipeline: wgpu::RenderPipeline,
    quad_buffer: wgpu::Buffer,
    quad_capacity: usize,

    glyph_pipeline: wgpu::RenderPipeline,
    glyph_buffer: wgpu::Buffer,
    glyph_capacity: usize,
    atlas_texture: wgpu::Texture,
    atlas_bind_group: wgpu::BindGroup,

    backend_info: BackendInfo,
    viewport: [f32; 2],
}

impl Renderer {
    const INITIAL_QUADS: usize = 1024;
    const INITIAL_GLYPHS: usize = 8192;

    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        backend_info: BackendInfo,
        atlas_size: u32,
    ) -> Self {
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("zero.globals.layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let globals_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("zero.globals"),
            contents: bytemuck::bytes_of(&Globals { viewport: [1.0, 1.0], _padding: [0.0; 2] }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("zero.globals.bind"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
        });

        // --- Quads -----------------------------------------------------------
        let quad_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("zero.quad.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/quad.wgsl").into()),
        });

        let quad_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("zero.quad.pipeline.layout"),
            bind_group_layouts: &[Some(&globals_layout)],
            immediate_size: 0,
        });

        let quad_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("zero.quad.pipeline"),
            layout: Some(&quad_layout),
            vertex: wgpu::VertexState {
                module: &quad_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<QuadInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x4,  // rect
                        1 => Float32x4,  // background
                        2 => Float32x4,  // border colour
                        3 => Float32x4,  // clip rect
                        4 => Float32x2,  // border width, corner radius
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &quad_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(PREMULTIPLIED_BLEND),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let quad_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("zero.quad.instances"),
            size: (Self::INITIAL_QUADS * std::mem::size_of::<QuadInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // --- Glyphs ----------------------------------------------------------
        let atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("zero.glyph.atlas"),
            size: wgpu::Extent3d {
                width: atlas_size,
                height: atlas_size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let atlas_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("zero.glyph.sampler"),
            // Nearest: glyph quads are placed on whole physical pixels, so
            // filtering would only soften edges that are already correct.
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("zero.glyph.atlas.layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
            ],
        });

        let atlas_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("zero.glyph.atlas.bind"),
            layout: &atlas_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                },
            ],
        });

        let glyph_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("zero.glyph.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/glyph.wgsl").into()),
        });

        let glyph_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("zero.glyph.pipeline.layout"),
            bind_group_layouts: &[Some(&globals_layout), Some(&atlas_layout)],
            immediate_size: 0,
        });

        let glyph_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("zero.glyph.pipeline"),
            layout: Some(&glyph_layout),
            vertex: wgpu::VertexState {
                module: &glyph_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GlyphInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x4,  // rect
                        1 => Float32x4,  // uv
                        2 => Float32x4,  // colour
                        3 => Float32x4,  // clip rect
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &glyph_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(PREMULTIPLIED_BLEND),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let glyph_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("zero.glyph.instances"),
            size: (Self::INITIAL_GLYPHS * std::mem::size_of::<GlyphInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            device,
            queue,
            surface_format,
            globals_buffer,
            globals_bind_group,
            quad_pipeline,
            quad_buffer,
            quad_capacity: Self::INITIAL_QUADS,
            glyph_pipeline,
            glyph_buffer,
            glyph_capacity: Self::INITIAL_GLYPHS,
            atlas_texture,
            atlas_bind_group,
            backend_info,
            viewport: [1.0, 1.0],
        }
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface_format
    }

    pub fn backend_info(&self) -> &BackendInfo {
        &self.backend_info
    }

    /// Draw a scene into `target`.
    ///
    /// `damage` is currently used to decide whether to draw at all; a finer
    /// scissor pass will narrow it to the changed region, which is why it is
    /// threaded through now rather than added later.
    pub fn render(
        &mut self,
        target: &wgpu::TextureView,
        scene: &Scene,
        text: &mut TextSystem,
        viewport: Rect,
        clear: z_tokens::Rgba,
        damage: Option<Rect>,
    ) -> FrameStats {
        if damage.is_none() && !scene.is_empty() {
            return FrameStats { quads: 0, glyphs: 0, draw_calls: 0, skipped: true };
        }

        self.set_viewport(viewport.width, viewport.height);
        self.upload_atlas(text);

        // Build the instance data layer by layer so each layer's quads and its
        // glyphs are contiguous. Drawing all quads first would be one call
        // cheaper, but an overlay panel could then be shown through by the
        // content text underneath it.
        let mut quads: Vec<QuadInstance> = Vec::new();
        let mut glyphs: Vec<GlyphInstance> = Vec::new();
        let mut batches: Vec<(std::ops::Range<u32>, std::ops::Range<u32>)> = Vec::new();

        for layer in crate::scene::Layer::ALL {
            let quad_start = quads.len() as u32;
            for quad in scene.quads_in(*layer) {
                quads.push(QuadInstance {
                    rect: [quad.bounds.x, quad.bounds.y, quad.bounds.width, quad.bounds.height],
                    background: quad.background.to_linear_premultiplied(),
                    border_color: quad.border_color.to_linear_premultiplied(),
                    clip: [quad.clip.x, quad.clip.y, quad.clip.right(), quad.clip.bottom()],
                    params: [quad.border_width, quad.corner_radius],
                });
            }

            let glyph_start = glyphs.len() as u32;
            for run in scene.texts_in(*layer) {
                for glyph in text.layout(run) {
                    glyphs.push(GlyphInstance {
                        rect: [
                            glyph.bounds.x,
                            glyph.bounds.y,
                            glyph.bounds.width,
                            glyph.bounds.height,
                        ],
                        uv: glyph.uv,
                        color: glyph.color.to_linear_premultiplied(),
                        clip: [run.clip.x, run.clip.y, run.clip.right(), run.clip.bottom()],
                    });
                }
            }

            batches.push((quad_start..quads.len() as u32, glyph_start..glyphs.len() as u32));
        }

        // Shaping added glyphs to the atlas; upload before they are sampled.
        self.upload_atlas(text);

        Self::ensure_capacity(
            &self.device,
            &mut self.quad_buffer,
            &mut self.quad_capacity,
            quads.len(),
            std::mem::size_of::<QuadInstance>(),
            "zero.quad.instances",
        );
        Self::ensure_capacity(
            &self.device,
            &mut self.glyph_buffer,
            &mut self.glyph_capacity,
            glyphs.len(),
            std::mem::size_of::<GlyphInstance>(),
            "zero.glyph.instances",
        );

        if !quads.is_empty() {
            self.queue.write_buffer(&self.quad_buffer, 0, bytemuck::cast_slice(&quads));
        }
        if !glyphs.is_empty() {
            self.queue.write_buffer(&self.glyph_buffer, 0, bytemuck::cast_slice(&glyphs));
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("zero.frame") });

        let mut draw_calls = 0;
        {
            let [r, g, b, a] = clear.to_linear_premultiplied();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("zero.shell.pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: r as f64,
                            g: g as f64,
                            b: b as f64,
                            a: a as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            for (quad_range, glyph_range) in &batches {
                if !quad_range.is_empty() {
                    pass.set_pipeline(&self.quad_pipeline);
                    pass.set_bind_group(0, &self.globals_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.quad_buffer.slice(..));
                    pass.draw(0..4, quad_range.clone());
                    draw_calls += 1;
                }
                if !glyph_range.is_empty() {
                    pass.set_pipeline(&self.glyph_pipeline);
                    pass.set_bind_group(0, &self.globals_bind_group, &[]);
                    pass.set_bind_group(1, &self.atlas_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.glyph_buffer.slice(..));
                    pass.draw(0..4, glyph_range.clone());
                    draw_calls += 1;
                }
            }
        }

        self.queue.submit(Some(encoder.finish()));

        FrameStats {
            quads: quads.len() as u32,
            glyphs: glyphs.len() as u32,
            draw_calls,
            skipped: false,
        }
    }

    fn set_viewport(&mut self, width: f32, height: f32) {
        let viewport = [width.max(1.0), height.max(1.0)];
        if viewport != self.viewport {
            self.viewport = viewport;
        }
        self.queue.write_buffer(
            &self.globals_buffer,
            0,
            bytemuck::bytes_of(&Globals { viewport: self.viewport, _padding: [0.0; 2] }),
        );
    }

    fn upload_atlas(&mut self, text: &mut TextSystem) {
        let atlas = text.atlas_mut();
        if !atlas.take_dirty() {
            return;
        }
        let (width, height) = (atlas.width(), atlas.height());
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.atlas_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            atlas.pixels(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
    }

    /// Grow an instance buffer when a frame needs more room.
    ///
    /// Growth doubles rather than fitting exactly, so a busy frame does not
    /// reallocate on every push — allocation in the frame path is the thing
    /// being avoided here.
    fn ensure_capacity(
        device: &wgpu::Device,
        buffer: &mut wgpu::Buffer,
        capacity: &mut usize,
        needed: usize,
        stride: usize,
        label: &str,
    ) {
        if needed <= *capacity {
            return;
        }
        let mut new_capacity = (*capacity).max(1);
        while new_capacity < needed {
            new_capacity *= 2;
        }
        *capacity = new_capacity;
        *buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: (new_capacity * stride) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
    }
}

/// Standard premultiplied-alpha blending. Both pipelines emit premultiplied
/// colour, so this is the only correct blend state for either.
const PREMULTIPLIED_BLEND: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_structs_are_gpu_safe_sizes() {
        assert_eq!(std::mem::size_of::<QuadInstance>() % 4, 0, "vertex strides are 4-byte aligned");
        assert_eq!(std::mem::size_of::<GlyphInstance>() % 4, 0);
        assert_eq!(std::mem::size_of::<Globals>() % 16, 0, "uniforms need 16-byte alignment");
    }

    /// Guards a bug that cost a debugging round: `vertex_attr_array!` derives
    /// each attribute's byte offset from the formats it is handed, not from the
    /// Rust struct. A padding field, or fields declared out of order, shifts
    /// every attribute after it and the shader reads garbage — with no compile
    /// error and no validation failure, just wrong pixels.
    #[test]
    fn instance_structs_have_no_padding_to_desync_the_attribute_offsets() {
        // rect + background + border + clip + params
        assert_eq!(
            std::mem::size_of::<QuadInstance>(),
            (4 + 4 + 4 + 4 + 2) * 4,
            "QuadInstance size does not match the sum of its declared attributes"
        );
        // rect + uv + colour + clip
        assert_eq!(
            std::mem::size_of::<GlyphInstance>(),
            (4 + 4 + 4 + 4) * 4,
            "GlyphInstance size does not match the sum of its declared attributes"
        );
    }
}
