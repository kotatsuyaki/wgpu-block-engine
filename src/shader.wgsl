struct VertexOutput {
    @location(0) color: vec3<f32>,
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn main_vs(
    @location(0) pos: vec3<f32>,
    @location(1) color: vec3<f32>
) -> VertexOutput {
    var out: VertexOutput;
    out.color = color;
    out.pos = vec4<f32>(pos, 1.0);
    return out;
}

@fragment
fn main_fs(vertex: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(vertex.color, 1.0);
}

// vim: set filetype=wgsl:
