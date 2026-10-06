//! WebGL binding adapter for the original liquid material.
use crate::materials::LiquidExt;
use bevy::{
    ecs::system::{lifetimeless::SRes, SystemParamItem},
    prelude::Vec4,
    render::{
        render_asset::RenderAssets,
        render_resource::{
            encase::{internal::WriteInto, ShaderType},
            AsBindGroup, AsBindGroupError, BindGroupLayout, BindGroupLayoutEntry, BindingResources,
            BindingType, BufferBindingType, OwnedBindingResource, OwnedData, SamplerBindingType,
            ShaderStages, TextureSampleType, TextureViewDimension, UnpreparedBindGroup,
        },
        texture::GpuImage,
    },
};

#[derive(Clone, Copy, ShaderType)]
struct LiquidParams {
    kind: Vec4,
    path: Vec4,
    anim: Vec4,
}

impl AsBindGroup for LiquidExt {
    type Data = ();
    type Param = SRes<RenderAssets<GpuImage>>;
    fn label() -> &'static str {
        "liquid_web_bind_group"
    }
    fn bind_group_data(&self) -> Self::Data {}
    fn unprepared_bind_group(
        &self,
        _layout: &BindGroupLayout,
        _device: &bevy::render::renderer::RenderDevice,
        images: &mut SystemParamItem<'_, '_, Self::Param>,
        _force_no_bindless: bool,
    ) -> Result<UnpreparedBindGroup, AsBindGroupError> {
        let frames = images
            .get(&self.frames)
            .ok_or(AsBindGroupError::RetryNextUpdate)?;
        let params = LiquidParams {
            kind: self.kind,
            path: self.path,
            anim: self.anim,
        };
        let mut bytes = Vec::new();
        params.write_into(
            &mut bevy::render::render_resource::encase::internal::Writer::new(
                &params, &mut bytes, 0,
            )
            .expect("LiquidParams uniform layout"),
        );
        bytes.resize(LiquidParams::min_size().get() as usize, 0);
        Ok(UnpreparedBindGroup {
            bindings: BindingResources(vec![
                (
                    90,
                    OwnedBindingResource::TextureView(
                        TextureViewDimension::D2,
                        self.light_buf.view().clone(),
                    ),
                ),
                (
                    100,
                    OwnedBindingResource::TextureView(
                        TextureViewDimension::D2Array,
                        frames.texture_view.clone(),
                    ),
                ),
                (
                    101,
                    OwnedBindingResource::Sampler(
                        SamplerBindingType::Filtering,
                        frames.sampler.clone(),
                    ),
                ),
                (102, OwnedBindingResource::Data(OwnedData(bytes))),
            ]),
        })
    }
    fn bind_group_layout_entries(
        _device: &bevy::render::renderer::RenderDevice,
        _force_no_bindless: bool,
    ) -> Vec<BindGroupLayoutEntry> {
        vec![
            BindGroupLayoutEntry {
                binding: 90,
                visibility: ShaderStages::VERTEX_FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Uint,
                    view_dimension: TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 100,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 101,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 102,
                visibility: ShaderStages::VERTEX_FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: Some(LiquidParams::min_size()),
                },
                count: None,
            },
        ]
    }
}
