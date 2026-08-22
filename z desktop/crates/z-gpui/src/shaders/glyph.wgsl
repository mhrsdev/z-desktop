// Glyph quads sampled from the shared coverage atlas.
//
// One instanced draw call for every glyph on screen. The atlas is R8 coverage
// produced by the shaper; colour comes from the instance, so the same cached
// glyph serves every colour it is ever drawn in.

struct Globals {
    viewport: vec2<f32>,
    _padding: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var atlas_texture: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;

struct Instance {
    @location(0) rect: vec4<f32>,   // x, y, width, height in logical pixels
    @location(1) uv: vec4<f32>,     // u0, v0, u1, v1 — normalised atlas coords
    @location(2) color: vec4<f32>,  // premultiplied linear
    @location(3) clip: vec4<f32>,   // x0, y0, x1, y1
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) clip: vec4<f32>,
    @location(3) world: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32, instance: Instance) -> VertexOut {
    let corner = vec2<f32>(
        f32(vertex_index & 1u),
        f32((vertex_index >> 1u) & 1u)
    );

    let position = instance.rect.xy + corner * instance.rect.zw;
    let ndc = vec2<f32>(
        position.x / globals.viewport.x * 2.0 - 1.0,
        1.0 - position.y / globals.viewport.y * 2.0
    );

    var out: VertexOut;
    out.clip_position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = mix(instance.uv.xy, instance.uv.zw, corner);
    out.color = instance.color;
    out.clip = instance.clip;
    out.world = position;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // Per-fragment so a glyph straddling the edge of a scroll area is cut,
    // not dropped whole.
    if (in.world.x < in.clip.x || in.world.x > in.clip.z ||
        in.world.y < in.clip.y || in.world.y > in.clip.w) {
        discard;
    }

    let coverage = textureSample(atlas_texture, atlas_sampler, in.uv).r;
    // Colour is already premultiplied, so scaling by coverage keeps it so.
    return in.color * coverage;
}
