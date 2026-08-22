// Rounded rectangles with borders, drawn as one instanced draw call.
//
// The entire structural layer of the shell — surfaces, cards, inputs, dividers,
// badges, focus rings — is this one primitive. Keeping it to a single pipeline
// is what makes the whole background layer cost one draw call instead of one
// per widget.
//
// Colours arrive premultiplied and in linear space; the pipeline blends with
// One / OneMinusSrcAlpha to match.

struct Globals {
    // Viewport in logical pixels. The scale factor is already folded into the
    // surface size, so shaders never see physical pixels.
    viewport: vec2<f32>,
    _padding: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct Instance {
    @location(0) rect: vec4<f32>,          // x, y, width, height
    @location(1) background: vec4<f32>,
    @location(2) border_color: vec4<f32>,
    @location(3) clip: vec4<f32>,          // x0, y0, x1, y1
    @location(4) params: vec2<f32>,        // border_width, corner_radius
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local: vec2<f32>,         // offset from the rect's centre
    @location(1) half_size: vec2<f32>,
    @location(2) background: vec4<f32>,
    @location(3) border_color: vec4<f32>,
    @location(4) params: vec2<f32>,
    @location(5) clip: vec4<f32>,
    @location(6) world: vec2<f32>,   // position in logical pixels
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32, instance: Instance) -> VertexOut {
    // Unit quad from a 4-vertex triangle strip: (0,0) (1,0) (0,1) (1,1).
    let corner = vec2<f32>(
        f32(vertex_index & 1u),
        f32((vertex_index >> 1u) & 1u)
    );

    // Grow by one pixel on every side so the antialiased edge has somewhere to
    // fade; without this the outer half-pixel of every rounded corner is clipped.
    let bleed = 1.0;
    let size = instance.rect.zw + vec2<f32>(bleed * 2.0, bleed * 2.0);
    let origin = instance.rect.xy - vec2<f32>(bleed, bleed);
    let position = origin + corner * size;

    // Logical pixels to clip space, with y pointing down.
    let ndc = vec2<f32>(
        position.x / globals.viewport.x * 2.0 - 1.0,
        1.0 - position.y / globals.viewport.y * 2.0
    );

    var out: VertexOut;
    out.clip_position = vec4<f32>(ndc, 0.0, 1.0);
    out.half_size = instance.rect.zw * 0.5;
    out.local = (corner - vec2<f32>(0.5, 0.5)) * size;
    out.background = instance.background;
    out.border_color = instance.border_color;
    out.params = instance.params;
    out.clip = instance.clip;
    out.world = position;
    return out;
}

// Signed distance to a rounded box: negative inside, positive outside.
fn rounded_box_sdf(point: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(point) - half_size + vec2<f32>(radius, radius);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // Clip in the shader rather than by shrinking the rect: shrinking would
    // drag the rounded corners inward, so a card cut off by a scroll edge
    // would grow a new corner instead of showing a straight cut.
    if (in.world.x < in.clip.x || in.world.x > in.clip.z ||
        in.world.y < in.clip.y || in.world.y > in.clip.w) {
        discard;
    }

    let border_width = in.params.x;
    // A radius larger than the shortest half-side would fold the corners
    // through each other, so clamp rather than trusting the caller.
    let radius = clamp(in.params.y, 0.0, min(in.half_size.x, in.half_size.y));

    let distance = rounded_box_sdf(in.local, in.half_size, radius);

    // Antialias across roughly one pixel of screen-space change.
    let edge = max(fwidth(distance), 0.0001);
    let outer_coverage = 1.0 - smoothstep(-edge, edge, distance);

    var color = in.background;
    if (border_width > 0.0) {
        // The border occupies the ring between the outer edge and an edge
        // inset by border_width.
        let inner_distance = distance + border_width;
        let inner_coverage = 1.0 - smoothstep(-edge, edge, inner_distance);
        color = mix(in.border_color, in.background, inner_coverage);
    }

    return color * outer_coverage;
}
