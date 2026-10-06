// WebGL2 model fallback: the production StandardMaterial texture/alpha path with classic matte
// lighting, fog and a small animated-material uniform. No storage tables or point-light loops.
#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_view_bindings::view,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
}

struct WebLight {
    ambient: vec4<f32>,
    diffuse: vec4<f32>,
    sun: vec4<f32>,
    specular: vec4<f32>,
    fog_color: vec4<f32>,
    fog_params: vec4<f32>,
};

struct ModelParams {
    tint: vec4<f32>,
    uv_anim: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(90) var<uniform> light: WebLight;
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> model: ModelParams;

fn safe_normalize(v: vec3<f32>) -> vec3<f32> {
    let length_squared = dot(v, v);
    return select(vec3<f32>(0.0, 1.0, 0.0), normalize(v), length_squared > 1e-12);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> @location(0) vec4<f32> {
    var animated = in;
#ifdef VERTEX_UVS_A
    animated.uv = in.uv + model.uv_anim.xy;
#endif
    let pbr_input = pbr_input_from_standard_material(animated, is_front);
    let base = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    let normal = safe_normalize(select(-pbr_input.world_normal, pbr_input.world_normal, is_front));
    let to_light = -safe_normalize(light.sun.xyz);
    let diffuse = max(dot(normal, to_light), 0.0);
    let directional = clamp(
        light.ambient.rgb + light.diffuse.rgb * diffuse,
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    );
    // `tint.a = 0` is the compact browser lane's fullbright bit. RGB remains the authored/static
    // tint, so this costs no extra uniform binding and preserves the existing G2 probe layout.
    let lighting = mix(vec3<f32>(1.0), directional, clamp(model.tint.a, 0.0, 1.0));
    var color = base.rgb * model.tint.rgb * lighting;

    if (light.fog_color.w > 0.5) {
        let eye_z = -(view.view_from_world * vec4<f32>(in.world_position.xyz, 1.0)).z;
        let fog_range = max(light.fog_params.y - light.fog_params.x, 0.001);
        let fog_factor = clamp((light.fog_params.y - eye_z) / fog_range, 0.0, 1.0);
        color = mix(light.fog_color.rgb, color, fog_factor);
    }
    return vec4<f32>(color, base.a);
}
