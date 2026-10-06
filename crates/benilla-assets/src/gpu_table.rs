//! Target-owned GPU table used by Benilla's custom renderers.
//!
//! Native/WebGPU keeps the existing storage-buffer representation.  WebGL2
//! stores the same bytes in an `rgba32uint` texture and shaders recover float
//! rows with `bitcast`.  The CPU owners therefore keep their existing packed
//! layout and offset/range writes; only this transport knows which GPU resource
//! is underneath.

use bevy::render::renderer::{RenderDevice, RenderQueue};

#[cfg(not(target_arch = "wasm32"))]
use bevy::render::render_resource::{Buffer, BufferDescriptor, BufferId, BufferUsages};

#[cfg(not(target_arch = "wasm32"))]
pub type GpuTable = Buffer;

#[cfg(not(target_arch = "wasm32"))]
pub type GpuTableId = BufferId;

#[cfg(not(target_arch = "wasm32"))]
pub fn create_gpu_table(device: &RenderDevice, size: u64, label: &'static str) -> GpuTable {
    device.create_buffer(&BufferDescriptor {
        label: Some(label),
        size,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub fn gpu_table_id(table: &GpuTable) -> GpuTableId {
    table.id()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn write_gpu_table(queue: &RenderQueue, table: &GpuTable, offset: u64, bytes: &[u8]) {
    queue.write_buffer(table, offset, bytes);
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::sync::{Arc, Mutex};

    use bevy::render::render_resource::{
        Extent3d, Origin3d, TexelCopyBufferLayout, TexelCopyTextureInfo, Texture, TextureAspect,
        TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureView,
        TextureViewDescriptor,
    };

    use super::*;

    const TEXEL_BYTES: usize = 16;
    // WebGL2 guarantees at least 2048 for MAX_TEXTURE_SIZE.  Keeping the width
    // fixed makes the WGSL address calculation stable across adapters.
    pub const GPU_TABLE_WIDTH: u32 = 2048;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct GpuTableId(u32);

    impl GpuTableId {
        fn next() -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static NEXT: AtomicU32 = AtomicU32::new(1);
            Self(NEXT.fetch_add(1, Ordering::Relaxed))
        }
    }

    /// Raw WebGL2 table texture plus its CPU mirror for sub-texel writes.
    ///
    /// Some production tables update one `u32` at a time while the texture's
    /// addressable unit is an RGBA texel.  The mirror preserves the other three
    /// words when such a range is uploaded.
    #[derive(Clone, Debug)]
    pub struct GpuTable {
        id: GpuTableId,
        texture: Texture,
        view: TextureView,
        size: Extent3d,
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl GpuTable {
        pub fn id(&self) -> GpuTableId {
            self.id
        }

        pub fn view(&self) -> &TextureView {
            &self.view
        }
    }

    pub fn create_gpu_table(device: &RenderDevice, size: u64, label: &'static str) -> GpuTable {
        let byte_len = usize::try_from(size).expect("GPU table fits address space");
        let texels = byte_len.div_ceil(TEXEL_BYTES).max(1);
        let width = texels.min(GPU_TABLE_WIDTH as usize) as u32;
        let height = texels.div_ceil(width as usize) as u32;
        let extent = Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&TextureDescriptor {
            label: Some(label),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba32Uint,
            usage: TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());
        GpuTable {
            id: GpuTableId::next(),
            texture,
            view,
            size: extent,
            bytes: Arc::new(Mutex::new(vec![0; texels * TEXEL_BYTES])),
        }
    }

    pub fn gpu_table_id(table: &GpuTable) -> GpuTableId {
        table.id()
    }

    pub fn write_gpu_table(queue: &RenderQueue, table: &GpuTable, offset: u64, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let start = usize::try_from(offset).expect("GPU table offset fits address space");
        let end = start
            .checked_add(bytes.len())
            .expect("GPU table write overflow");
        let mut mirror = table.bytes.lock().unwrap_or_else(|e| e.into_inner());
        assert!(end <= mirror.len(), "GPU table write exceeds allocation");
        mirror[start..end].copy_from_slice(bytes);

        let first_texel = start / TEXEL_BYTES;
        let end_texel = end.div_ceil(TEXEL_BYTES);
        let row_width = table.size.width as usize;
        let mut texel = first_texel;
        while texel < end_texel {
            let x = texel % row_width;
            let y = texel / row_width;
            let remaining = end_texel - texel;
            // The producers already coalesce dirty ranges. Do not split their
            // complete rows back into one staging allocation/command per row:
            // upload the full-row middle as one rectangle, with at most one
            // partial row on either side. No gaps or extra texels are uploaded.
            let (width, height) = if x == 0 && remaining >= row_width {
                (row_width, remaining / row_width)
            } else {
                (remaining.min(row_width - x), 1)
            };
            let count = width * height;
            let byte_start = texel * TEXEL_BYTES;
            let byte_end = byte_start + count * TEXEL_BYTES;
            queue.write_texture(
                TexelCopyTextureInfo {
                    texture: &table.texture,
                    mip_level: 0,
                    origin: Origin3d {
                        x: x as u32,
                        y: y as u32,
                        z: 0,
                    },
                    aspect: TextureAspect::All,
                },
                &mirror[byte_start..byte_end],
                TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some((width * TEXEL_BYTES) as u32),
                    rows_per_image: Some(height as u32),
                },
                Extent3d {
                    width: width as u32,
                    height: height as u32,
                    depth_or_array_layers: 1,
                },
            );
            texel += count;
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::{
    create_gpu_table, gpu_table_id, write_gpu_table, GpuTable, GpuTableId, GPU_TABLE_WIDTH,
};
