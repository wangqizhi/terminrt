struct Uniforms {
    screen_size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

fn pixel_to_ndc(pos: vec2<f32>) -> vec4<f32> {
    let x = (pos.x / uniforms.screen_size.x) * 2.0 - 1.0;
    let y = 1.0 - (pos.y / uniforms.screen_size.y) * 2.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

struct ColorVSIn {
    @location(0) pos: vec2<f32>,
    @location(1) color: vec4<f32>,
};

struct ColorVSOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_color(input: ColorVSIn) -> ColorVSOut {
    var out: ColorVSOut;
    out.pos = pixel_to_ndc(input.pos);
    out.color = input.color;
    return out;
}

@fragment
fn fs_color(input: ColorVSOut) -> @location(0) vec4<f32> {
    return input.color;
}

struct GlyphVSIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
};

struct GlyphVSOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(1)
var glyph_tex: texture_2d<f32>;

@group(0) @binding(2)
var glyph_sampler: sampler;

@vertex
fn vs_glyph(input: GlyphVSIn) -> GlyphVSOut {
    var out: GlyphVSOut;
    out.pos = pixel_to_ndc(input.pos);
    out.uv = input.uv;
    return out;
}

@fragment
fn fs_glyph(input: GlyphVSOut) -> @location(0) vec4<f32> {
    let a = textureSample(glyph_tex, glyph_sampler, input.uv).r;
    return vec4<f32>(1.0, 1.0, 1.0, a);
}
