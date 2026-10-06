#import bevy_sprite::mesh2d_vertex_output::VertexOutput

struct ProbeLight {
    ambient: vec4<f32>,
    sunlight: vec4<f32>,
    fog: vec4<f32>,
    params: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var layers: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var layers_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var alpha: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var alpha_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var shadow: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var shadow_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var<uniform> light: ProbeLight;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let layer_uv = in.world_position.xy / light.params.x;

    var weights = vec4<f32>(
        textureSample(alpha, alpha_sampler, in.uv, 0).r,
        textureSample(alpha, alpha_sampler, in.uv, 1).r,
        textureSample(alpha, alpha_sampler, in.uv, 2).r,
        textureSample(alpha, alpha_sampler, in.uv, 3).r,
    );
    weights /= max(dot(weights, vec4<f32>(1.0)), 0.0001);

    var albedo = textureSample(layers, layers_sampler, layer_uv, 0).rgb * weights.x;
    albedo += textureSample(layers, layers_sampler, layer_uv, 1).rgb * weights.y;
    albedo += textureSample(layers, layers_sampler, layer_uv, 2).rgb * weights.z;
    albedo += textureSample(layers, layers_sampler, layer_uv, 3).rgb * weights.w;

    let wave = sin(in.world_position.x * 0.018 + light.fog.w * 0.45)
        * cos(in.world_position.y * 0.014 - light.fog.w * 0.31);
    let diffuse = clamp(0.62 + wave * 0.25, 0.22, 1.0);
    let shadow_uv = in.uv + vec2<f32>(light.fog.w * 0.003, 0.0);
    let shadow_amount = textureSample(shadow, shadow_sampler, shadow_uv, 0).r;
    let baked_shadow = 1.0 - shadow_amount * light.params.y;
    let lighting = light.ambient.rgb * light.ambient.a
        + light.sunlight.rgb * light.sunlight.a * diffuse;

    // World-space grid: camera movement must pan across it rather than leaving it screen-locked.
    let cell = abs(fract(in.world_position.xy / light.params.x) - vec2<f32>(0.5));
    let grid = smoothstep(0.455, 0.49, max(cell.x, cell.y));
    var color = albedo * lighting * baked_shadow;
    color = mix(color, color * 0.38, grid * 0.55);

    let distance_from_center = length(in.world_position.xy / vec2<f32>(900.0, 550.0));
    let fog_amount = smoothstep(light.params.z, light.params.w, distance_from_center);
    color = mix(color, light.fog.rgb, fog_amount * 0.82);

    return vec4<f32>(color, 1.0);
}
