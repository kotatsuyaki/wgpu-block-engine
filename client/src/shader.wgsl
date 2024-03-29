struct VertexOutput {
    @location(1) texcoord: vec2<f32>,
    @location(2) brightness: f32,
    @builtin(position) pos: vec4<f32>,
};

struct UniformData {
    trans: mat4x4<f32>,
};

struct PushConstantsData {
    shift: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> uniform_data: UniformData;

@group(1) @binding(0)
var grass_texture: texture_2d<f32>;
@group(1) @binding(1)
var grass_sampler: sampler;

var<push_constant> pc: PushConstantsData;

@vertex
fn main_vs(
    @location(0) pos: vec3<f32>,
    @location(1) texcoord: vec2<f32>,
    @location(2) brightness: f32
) -> VertexOutput {
    var out: VertexOutput;

    out.texcoord = texcoord;

    out.pos = vec4<f32>(pos, 1.0);
    out.pos = uniform_data.trans * (out.pos + pc.shift);

    out.brightness = brightness;

    return out;
}


@fragment
fn main_fs(vertex: VertexOutput) -> @location(0) vec4<f32> {
    let grass_multiplier = vec4<f32>(0.5, 0.76, 0.26, 1.0);
    return grass_multiplier * textureSample(grass_texture, grass_sampler, vertex.texcoord) * vertex.brightness;
}

// vim: set filetype=wgsl:
