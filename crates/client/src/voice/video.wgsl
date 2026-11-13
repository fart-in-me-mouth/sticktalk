@group(0) @binding(0) var video: texture_2d<f32>;
@group(0) @binding(1) var filtering: sampler;
struct Vertex { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs(@builtin(vertex_index) id: u32) -> Vertex {
    let uv = vec2<f32>(f32((id << 1u) & 2u), f32(id & 2u));
    return Vertex(vec4<f32>(uv * vec2<f32>(2., -2.) + vec2<f32>(-1.,1.), 0.,1.), uv);
}
@fragment fn fs(v: Vertex) -> @location(0) vec4<f32> {
    // Iced's linear render target expects linear RGB; decoded video is BT.709/sRGB.
    let rgb = textureSample(video, filtering, v.uv).rgb;
    let lo = rgb / 12.92;
    let hi = pow((rgb + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return vec4<f32>(select(hi, lo, rgb <= vec3<f32>(0.04045)),1.);
}
