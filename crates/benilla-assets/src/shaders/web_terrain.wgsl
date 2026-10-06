// WebGL2 terrain fallback. Texture bindings and mesh attributes intentionally mirror terrain.wgsl;
// only the unbounded storage-buffer features (point lights/probes/tables) are absent.
#import bevy_pbr::{
    forward_io::Vertex,
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}

struct WebLight {
    ambient: vec4<f32>,
    diffuse: vec4<f32>,
    sun: vec4<f32>,
    specular: vec4<f32>,
    fog_color: vec4<f32>,
    fog_params: vec4<f32>,
};

struct TerrainParams {
    params: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(90) var<uniform> light: WebLight;
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var layer_array: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var alpha_array: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var splat_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var<uniform> terrain: TerrainParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(110) var shadow_array: texture_2d_array<f32>;

struct TerrainOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) uv_b: vec2<f32>,
    @location(4) layer_indices: vec4<f32>,
    @location(5) specular: vec3<f32>,
    @location(6) primary: vec3<f32>,
};

fn safe_normalize(v: vec3<f32>) -> vec3<f32> {
    let length_squared = dot(v, v);
    return select(vec3<f32>(0.0, 1.0, 0.0), normalize(v), length_squared > 1e-12);
}

@vertex
fn vertex(in: Vertex) -> TerrainOutput {
    var out: TerrainOutput;
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    out.world_position =
        mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(in.position, 1.0));
    out.clip_position = position_world_to_clip(out.world_position.xyz);
    let normal = safe_normalize(
        mesh_functions::mesh_normal_local_to_world(in.normal, in.instance_index),
    );
    let to_light = -safe_normalize(light.sun.xyz);
    let to_eye = safe_normalize(view.world_position.xyz - out.world_position.xyz);
    let halfway = safe_normalize(to_light + to_eye);
    let diffuse = max(dot(normal, to_light), 0.0);
    let sheen = pow(max(dot(normal, halfway), 0.0), light.specular.w);

    out.uv = in.uv;
    out.uv_b = in.uv_b;
    out.layer_indices = in.color;
    out.primary = clamp(light.ambient.rgb + light.diffuse.rgb * diffuse, vec3<f32>(0.0), vec3<f32>(1.0));
    out.specular = clamp(light.specular.rgb * sheen, vec3<f32>(0.0), vec3<f32>(1.0));
    return out;
}

@fragment
fn fragment(in: TerrainOutput) -> @location(0) vec4<f32> {
    let eye_z = -(view.view_from_world * vec4<f32>(in.world_position.xyz, 1.0)).z;
    if (light.fog_params.w > 0.0 && eye_z > light.fog_params.w) {
        discard;
    }

    let indices = vec4<i32>(round(in.layer_indices));
    let tiled_uv = in.uv * terrain.params.x;
    let layer0 = textureSample(layer_array, splat_sampler, tiled_uv, indices.x);
    let layer1 = textureSample(layer_array, splat_sampler, tiled_uv, indices.y);
    let layer2 = textureSample(layer_array, splat_sampler, tiled_uv, indices.z);
    let layer3 = textureSample(layer_array, splat_sampler, tiled_uv, indices.w);

    let array_uv = clamp(in.uv, vec2<f32>(0.5 / 64.0), vec2<f32>(1.0 - 0.5 / 64.0));
    let blend = textureSample(alpha_array, splat_sampler, array_uv, i32(round(in.uv_b.x)));
    var albedo = layer0.rgb;
    albedo = mix(albedo, layer1.rgb, blend.r);
    albedo = mix(albedo, layer2.rgb, blend.g);
    albedo = mix(albedo, layer3.rgb, blend.b);
    var sheen_mask = layer0.a;
    sheen_mask = mix(sheen_mask, layer1.a, blend.r);
    sheen_mask = mix(sheen_mask, layer2.a, blend.g);
    sheen_mask = mix(sheen_mask, layer3.a, blend.b);

    var shadow_lit = 1.0;
    if (in.uv_b.y >= 0.0) {
        let shadow = textureSample(
            shadow_array,
            splat_sampler,
            array_uv,
            i32(round(in.uv_b.y)),
        ).r;
        shadow_lit = 1.0 - shadow;
    }

    let diffuse = albedo * in.primary * (0.3 * shadow_lit + 0.7);
    let specular = in.specular * sheen_mask * shadow_lit;
    var color = clamp(diffuse + specular, vec3<f32>(0.0), vec3<f32>(1.0));
    if (light.fog_color.w > 0.5) {
        let fog_range = max(light.fog_params.y - light.fog_params.x, 0.001);
        let fog_factor = clamp((light.fog_params.y - eye_z) / fog_range, 0.0, 1.0);
        color = mix(light.fog_color.rgb, color, fog_factor);
    }
    return vec4<f32>(color, 1.0);
}
