//! GPU data rendering via wgpu.
//!
//! wgpu owns the window surface; the CPU raster stack (tiny-skia + swash)
//! rasterises axes / titles / labels and uploads them as a texture. A single
//! render pass draws data primitives followed by the chrome texture overlay,
//! then presents.

use crate::color::Color;
use crate::config::Config;
use crate::data_config::ScatterShape;
use crate::layout::Rect;

use wgpu::util::DeviceExt;

mod surface;
pub use surface::*;
mod textured;
pub use textured::*;
mod transform;
pub use transform::*;
mod style;
pub use style::*;
mod value_map;
pub use value_map::*;
mod selection;
pub(crate) use selection::*;
mod line;
pub use line::*;
mod stylized;
pub use stylized::*;
mod scatter;
pub use scatter::*;
mod errorbar;
pub use errorbar::*;
mod bar;
pub use bar::*;
mod field;
pub use field::*;
mod contour;
pub use contour::*;
mod composite;
pub use composite::*;
// The panel draw in slot order lives with the compositor; the name stays
// here for hosts that issue panels themselves.
pub use crate::renderer::draw_chart_panel_columnar;
pub mod bar_envelope;
pub mod column_pool;
pub mod line_arc;
pub(crate) mod stream_field;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod stream_point_style_tests;
pub mod vector;
pub use column_pool::{
    AllocError, BOX_PLOT_RECORD, ColumnHandle, ColumnId, ColumnPool, ColumnSlot, DefragPolicy,
    FreeRegion, GpuAllocCtx, GpuBudget, GrowthPolicy, MAX_RAGGED_VALUES, MAX_RECORD_FIELDS,
    RaggedId, RaggedRef, RaggedSlot, RaggedSource, RaggedStats, RecordId, RecordRef, RecordSchema,
    RecordSlot, RecordSource, RecordStats, StackExtent,
};

// Instance, adapter, surface, and device setup.

// Texture upload, samplers, bind groups, and pipelines for the chrome overlay.

// Transform uniform (data-space -> NDC), shared by scatter / line / errorbar.
// Vertex buffers carry data-space values directly; resize/zoom only updates
// this uniform, never the instance data.

// WGSL mirror size guards. Field order and size must remain byte-identical to
// every shader common block before either CPU structure changes.
const _: () = assert!(std::mem::size_of::<ScatterTransform>() == 144);

// Primitive style uniform — color and per-primitive options.
// Bind groups: group(0) = transform (shared), group(1) = style (per primitive).

// Columnar pipelines backed by ColumnPool.
//
// Every column is uploaded once into a ColumnPool buffer; charts draw by
// binding `pool.buffer().slice(handle.byte_range())` into vertex slots.
// Shaders take a single column per slot; lengths and offsets are decided by
// the caller via the slice and draw range.

// Procedural planet-atlas bakes run once per style-set creation; results are
// cached on the GPU and never recomputed per frame.

// Tests.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contour_lookup_uses_finite_prefixes_numeric_indices_and_block_metadata() {
        let mut levels: Vec<f32> = (0..35).map(|index| 100.0 - index as f32).collect();
        levels[2] = 50.0;
        levels[3] = 50.0;
        levels[4] = f32::NAN;
        levels[30] = f32::NEG_INFINITY;
        levels[32] = f32::NEG_INFINITY;
        levels[33] = f32::INFINITY;
        let ramp = [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]];
        let lookup = build_field_lookup_tables(&ramp, &levels).unwrap();

        assert_eq!(&lookup.stops[..ramp.len()], &ramp);
        assert_eq!(lookup.stops.len(), ramp.len() + levels.len());
        assert_eq!(
            lookup.metadata,
            vec![
                ContourLookupMetadataGpu {
                    finite_count: 30,
                    negative_infinity_count: 1,
                },
                ContourLookupMetadataGpu {
                    finite_count: 1,
                    negative_infinity_count: 1,
                },
            ]
        );

        let search = &lookup.stops[ramp.len()..];
        let first_finite = &search[..lookup.metadata[0].finite_count as usize];
        assert!(first_finite.windows(2).all(|pair| pair[0][0] <= pair[1][0]));
        assert!(search[30..32].iter().all(|record| *record == [0.0; 4]));
        assert_eq!(search[32][0], levels[34]);
        assert_eq!(search[32][1], 34.0);
        assert!(search[33..35].iter().all(|record| *record == [0.0; 4]));

        let duplicate_indices: Vec<u32> = first_finite
            .iter()
            .filter(|record| record[0] == 50.0)
            .map(|record| record[1] as u32)
            .collect();
        assert_eq!(duplicate_indices, vec![2, 3]);
        for record in first_finite {
            let original = record[1] as usize;
            assert_eq!(record[1], original as f32);
            assert_eq!(record[0], levels[original]);
        }
    }

    #[test]
    fn contour_lookup_pads_empty_metadata_and_preserves_31_32_1023_indices() {
        let empty = build_field_lookup_tables(&[[0.0; 4]], &[]).unwrap();
        assert_eq!(empty.stops, vec![[0.0; 4]]);
        assert_eq!(empty.metadata, vec![ContourLookupMetadataGpu::default()]);

        let allowed = vec![0.0; CONTOUR_LEVEL_LOOKUP_CAPACITY];
        let lookup = build_field_lookup_tables(&[[0.0; 4]], &allowed).unwrap();
        assert_eq!(lookup.stops.len(), 1 + CONTOUR_LEVEL_LOOKUP_CAPACITY);
        assert_eq!(lookup.metadata.len(), CONTOUR_LEVEL_BLOCK_COUNT);
        let search = &lookup.stops[1..];
        for index in [31usize, 32, 1023] {
            let block = index / CONTOUR_LEVEL_BLOCK_SIZE;
            let start = block * CONTOUR_LEVEL_BLOCK_SIZE;
            let end = start + lookup.metadata[block].finite_count as usize;
            let record = search[start..end]
                .iter()
                .find(|record| record[1] == index as f32)
                .unwrap_or_else(|| panic!("numeric original index {index} was not preserved"));
            assert_eq!(record[1].to_bits(), (index as f32).to_bits());
        }

        let rejected = vec![0.0; CONTOUR_LEVEL_LOOKUP_CAPACITY + 1];
        assert!(matches!(
            build_field_lookup_tables(&[[0.0; 4]], &rejected),
            Err(crate::FiggyError::StateAllocationFailed {
                resource: "contour level lookup",
                ..
            })
        ));
    }

    #[test]
    fn contour_lookup_dimensions_match_the_model_and_shader() {
        assert_eq!(CONTOUR_LEVEL_BLOCK_SIZE, 32);
        assert_eq!(CONTOUR_LEVEL_BLOCK_COUNT, 32);
        assert_eq!(
            CONTOUR_LEVEL_LOOKUP_CAPACITY,
            crate::data_config::MAX_CONTOUR_LEVELS
        );

        let shader = FIELD_SHADER.source;
        assert!(shader.contains("const CONTOUR_LEVEL_BLOCK_SIZE: u32 = 32u;"));
        assert!(shader.contains("let original = firstTrailingBit(candidates);"));
        assert!(shader.contains("let original = u32(record.y) - start;"));
        assert!(shader.contains("contour_lookup_metadata[block].finite_count"));
        assert!(shader.contains("metadata.negative_infinity_count"));
        assert!(shader.contains("let search_lo = s.z_lo - margin;"));
        assert!(shader.contains("let search_hi = s.z_hi + margin;"));
        assert!(shader.contains("contour_lower_bound(start, count, search_lo)"));
        assert!(shader.contains("contour_upper_bound(start, count, search_hi)"));
        assert!(
            !shader.contains("for (var i = 0u; i < field.level_count"),
            "contour fragments must not return to a full level scan"
        );
    }

    #[test]
    fn field_table_exact_charge_includes_zero_level_lookup_metadata_padding() {
        let Some((device, _queue)) = shared_device() else {
            return;
        };
        let ledger = std::sync::Arc::new(crate::gpu_memory::GpuLedger::new());
        let layout = create_field_data_bind_group_layout(&device);
        let pool = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("field table charge test pool"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let lookup = build_field_lookup_tables(&[[0.0; 4]], &[]).unwrap();
        let params = <FieldParamsGpu as bytemuck::Zeroable>::zeroed();
        let (_bind_group, charge) = create_field_data_bind_group(
            &device,
            &ledger,
            &layout,
            &pool,
            FieldTables {
                grid: &[GridColumnGpu { base: 0, len: 0 }],
                levels: &[0.0],
                stops: &lookup.stops,
                level_colors: &[[0.0; 4]],
                lookup_metadata: &lookup.metadata,
                params: &params,
            },
        );

        let expected = std::mem::size_of::<GridColumnGpu>()
            + std::mem::size_of::<f32>()
            + std::mem::size_of::<[f32; 4]>()
            + std::mem::size_of::<[f32; 4]>()
            + std::mem::size_of::<ContourLookupMetadataGpu>()
            + std::mem::size_of::<FieldParamsGpu>();
        assert_eq!(expected, 116);
        assert_eq!(
            ledger
                .snapshot()
                .live_bytes_of(crate::gpu_memory::GpuResourceKind::FieldTable),
            expected as u64
        );
        drop(charge);
        assert_eq!(
            ledger
                .snapshot()
                .live_bytes_of(crate::gpu_memory::GpuResourceKind::FieldTable),
            0
        );
    }

    /// The constellation star picks on the disc its quad inscribes
    /// (SHADER_COMMON.md §5.7): the sprite is zero from 0.98 of the half
    /// width out, so even a bilinear tap a texel further stays inside it.
    #[test]
    fn psf_sprite_is_zero_outside_the_inscribed_disc() {
        let size = 128u32;
        let psf = bake_psf_rgba(size);
        let half = (size as f32 - 1.0) * 0.5;
        for y in 0..size {
            for x in 0..size {
                let dx = (x as f32 - half) / half;
                let dy = (y as f32 - half) / half;
                if (dx * dx + dy * dy).sqrt() >= 0.98 {
                    let i = ((y * size + x) * 4) as usize;
                    assert_eq!((psf[i], psf[i + 1]), (0, 0), "texel ({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn psf_sprite_fades_to_zero_at_square_edges() {
        let psf = bake_psf_rgba(128);
        let px = |x: usize, y: usize, c: usize| psf[(y * 128 + x) * 4 + c];

        for i in 0..128 {
            assert_eq!(px(i, 0, 0), 0, "top edge core leaked at x={i}");
            assert_eq!(px(i, 0, 1), 0, "top edge halo leaked at x={i}");
            assert_eq!(px(i, 127, 0), 0, "bottom edge core leaked at x={i}");
            assert_eq!(px(i, 127, 1), 0, "bottom edge halo leaked at x={i}");
            assert_eq!(px(0, i, 0), 0, "left edge core leaked at y={i}");
            assert_eq!(px(0, i, 1), 0, "left edge halo leaked at y={i}");
            assert_eq!(px(127, i, 0), 0, "right edge core leaked at y={i}");
            assert_eq!(px(127, i, 1), 0, "right edge halo leaked at y={i}");
        }
    }

    #[test]
    fn log_transform_guards_manual_range_without_raising_tiny_positive_min() {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        });
        config.chart_title.top_margin = 0.0;
        for axis in [
            &mut config.top_x,
            &mut config.bottom_x,
            &mut config.left_y,
            &mut config.right_y,
        ] {
            axis.out_margin = 0.0;
            axis.major_tick_length = 0.0;
        }

        config.bottom_x.scale = crate::config::AxisScale::Logarithmic;
        config.bottom_x.min = 0.0;
        config.bottom_x.max = 1000.0;
        let t = scatter_transform_from_config(&config);
        assert_eq!(t.scale_log[0], 1.0);
        assert!((t.data_min[0] + 12.0).abs() < 1.0e-6, "{:?}", t.data_min);
        assert!((t.data_max[0] - 3.0).abs() < 1.0e-6, "{:?}", t.data_max);

        config.bottom_x.min = 1.0e-15;
        config.bottom_x.max = 1.0e-12;
        let t = scatter_transform_from_config(&config);
        assert!((t.data_min[0] + 15.0).abs() < 1.0e-6, "{:?}", t.data_min);
        assert!((t.data_max[0] + 12.0).abs() < 1.0e-6, "{:?}", t.data_max);

        config.bottom_x.min = 10.0;
        config.bottom_x.max = 1.0;
        let t = scatter_transform_from_config(&config);
        assert!((t.data_min[0] - 1.0).abs() < 1.0e-6, "{:?}", t.data_min);
        assert!((t.data_max[0] - 2.0).abs() < 1.0e-6, "{:?}", t.data_max);
    }

    #[test]
    fn scatter_transform_inverted_axes_swap_final_ranges() {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 160,
            height: 120,
        });
        config.chart_title.top_margin = 0.0;
        for axis in [
            &mut config.top_x,
            &mut config.bottom_x,
            &mut config.left_y,
            &mut config.right_y,
        ] {
            axis.out_margin = 8.0;
            axis.major_tick_length = 0.0;
        }
        config.bottom_x.scale = crate::config::AxisScale::Logarithmic;
        config.bottom_x.min = 1.0;
        config.bottom_x.max = 100.0;
        config.left_y.min = -5.0;
        config.left_y.max = 5.0;

        let normal = scatter_transform_from_config(&config);
        config.bottom_x.inverted = true;
        config.left_y.inverted = true;
        let inverted = scatter_transform_from_config(&config);

        assert_eq!(inverted.scale_log, normal.scale_log);
        assert!((inverted.data_min[0] - normal.data_max[0]).abs() < 1.0e-6);
        assert!((inverted.data_max[0] - normal.data_min[0]).abs() < 1.0e-6);
        assert!((inverted.data_min[1] - normal.data_max[1]).abs() < 1.0e-6);
        assert!((inverted.data_max[1] - normal.data_min[1]).abs() < 1.0e-6);
        for v in inverted.data_min.into_iter().chain(inverted.data_max) {
            assert!(v.is_finite(), "{inverted:?}");
        }
    }

    #[test]
    fn gpu_transform_uses_auto_fit_ssot_bounds_bit_for_bit() {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 17,
            y: 29,
            width: 900,
            height: 600,
        });
        let x_extent = crate::chart::FitExtent {
            min: 1_700_000_000_000.125,
            max: 1_700_000_000_009.875,
            min_positive: Some(1_700_000_000_000.125),
        };
        let y_extent = crate::chart::FitExtent {
            min: -3.25,
            max: 8.75,
            min_positive: Some(0.125),
        };
        crate::chart::apply_auto_fit_all(&mut config, &x_extent, &y_extent, 0.05);

        let transform = scatter_transform_from_config(&config);
        let x_min = crate::data::split_f64_to_f32_pair(config.bottom_x.min);
        let x_max = crate::data::split_f64_to_f32_pair(config.bottom_x.max);
        let y_min = crate::data::split_f64_to_f32_pair(config.left_y.min);
        let y_max = crate::data::split_f64_to_f32_pair(config.left_y.max);
        assert_eq!(
            [transform.data_min[0], transform.data_min_lo[0]],
            [x_min.0, x_min.1]
        );
        assert_eq!(
            [transform.data_max[0], transform.data_max_lo[0]],
            [x_max.0, x_max.1]
        );
        assert_eq!(
            [transform.data_min[1], transform.data_min_lo[1]],
            [y_min.0, y_min.1]
        );
        assert_eq!(
            [transform.data_max[1], transform.data_max_lo[1]],
            [y_max.0, y_max.1]
        );

        // Layout changes only the panel affine. They can no longer manufacture
        // a second, margin-extended axis range behind Config's back.
        let mut different_layout = config.clone();
        different_layout.bottom_x.out_margin += 40.0;
        different_layout.left_y.out_margin += 30.0;
        let moved = scatter_transform_from_config(&different_layout);
        assert_eq!(moved.data_min, transform.data_min);
        assert_eq!(moved.data_max, transform.data_max);
        assert_eq!(moved.data_min_lo, transform.data_min_lo);
        assert_eq!(moved.data_max_lo, transform.data_max_lo);
        assert_ne!(moved.data_to_panel_offset, transform.data_to_panel_offset);
        assert_ne!(moved.data_to_panel_scale, transform.data_to_panel_scale);
    }

    /// Smoke-test the texture-upload API path (no readback). Validation
    /// would surface here if the call shape were wrong.
    #[test]
    fn rgba_texture_upload_roundtrips_api() {
        let Some((device, queue)) = shared_device() else {
            println!("no adapter — skipping texture upload test");
            return;
        };

        // 2x2 checkerboard: 4 RGBA pixels = 16 bytes.
        let rgba: [u8; 16] = [
            255, 0, 0, 255, // red
            0, 255, 0, 255, // green
            0, 0, 255, 255, // blue
            255, 255, 0, 255, // yellow
        ];
        let tex = upload_rgba_texture(
            &device,
            &queue,
            device.limits().max_texture_dimension_2d,
            2,
            2,
            &rgba,
        )
        .expect("upload texture");

        // Wait for the queued write_texture to complete; validation errors
        // surface during this poll.
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });

        assert_eq!(tex.width(), 2);
        assert_eq!(tex.height(), 2);
        assert_eq!(tex.format(), wgpu::TextureFormat::Rgba8Unorm);
    }

    /// Compile the WGSL and build the fullscreen textured pipeline. Shader
    /// or layout mismatches would panic during creation.
    #[test]
    fn fullscreen_textured_pipeline_compiles_and_creates() {
        let Some((device, _queue)) = shared_device() else {
            println!("no adapter — skipping pipeline test");
            return;
        };

        let bgl = create_texture_bind_group_layout(&device);
        let pipeline = create_fullscreen_textured_pipeline(
            &device,
            &bgl,
            // Same non-sRGB format the surface picks at runtime.
            wgpu::TextureFormat::Bgra8Unorm,
        );

        let _ = pipeline;
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
    }

    #[test]
    fn streamed_point_style_map_reuses_rows_and_charges_only_meta() {
        let (device, _queue) = shared_device().expect("streamed point style map GPU adapter");
        let layout = create_scatter_style_map_bind_group_layout(&device);
        let map = create_scatter_style_map(
            &device,
            &layout,
            &[ScatterStyleSlotGpu {
                color_premul: [1.0, 0.0, 0.0, 1.0],
                meta: [3.0, 0.0, 1.0, 0.0],
            }],
            &[ScatterStyleOverrideGpu {
                point_index: 17,
                _pad: [0; 3],
                color_premul: [0.0, 1.0, 0.0, 1.0],
                meta: [3.0, 0.0, 1.0, 0.0],
            }],
            ScatterStyleMapMeta {
                style_count: 1,
                override_count: 1,
                has_index: 1,
                _pad: 0,
            },
        );
        assert_eq!(map.meta._pad, 0);
        let tally = crate::gpu_memory::ChargeTally::new();
        let streamed = map.stream_bind_group(&device, &layout, 17, &tally);
        assert_eq!(tally.bytes(), 16);
        drop(map);
        let _ = streamed;
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
    }

    /// Compile the scatter WGSL entries added outside the common block for
    /// per-point style mapping and picked-point decoration. This catches WGSL
    /// syntax/layout drift without running a full render snapshot.
    #[test]
    fn scatter_mapped_and_pick_ring_pipelines_compile() {
        let Some((device, _queue)) = shared_device() else {
            println!("no adapter ??skipping scatter entry pipeline test");
            return;
        };
        let transform_bgl = create_scatter_transform_bind_group_layout(&device);
        let style_bgl = create_style_bind_group_layout(&device);
        let style_map_bgl = create_scatter_style_map_bind_group_layout(&device);
        let value_map_bgl = create_point_value_map_bind_group_layout(&device);
        let shaders = ShaderModules::new(&device);

        let mapped = create_scatter_columnar_mapped_pipeline(
            &device,
            &transform_bgl,
            &style_bgl,
            &style_map_bgl,
            &value_map_bgl,
            wgpu::TextureFormat::Bgra8Unorm,
            1,
        );
        let picked = create_scatter_columnar_pipeline_with_entries(
            &device,
            &shaders.scatter,
            &transform_bgl,
            &style_bgl,
            wgpu::TextureFormat::Bgra8Unorm,
            1,
            "vs_pick_ring",
            "fs_pick_ring",
            "figgy picked point ring pipeline test",
        );
        let picked_mapped = create_scatter_columnar_mapped_pipeline_with_entries(
            &device,
            &shaders.scatter,
            &transform_bgl,
            &style_bgl,
            &style_map_bgl,
            &value_map_bgl,
            wgpu::TextureFormat::Bgra8Unorm,
            1,
            "vs_pick_ring_mapped",
            "fs_pick_ring",
            "figgy picked point mapped ring pipeline test",
        );
        let errorbar_mapped = create_errorbar_columnar_mapped_pipeline(
            &device,
            &transform_bgl,
            &style_bgl,
            &style_map_bgl,
            wgpu::TextureFormat::Bgra8Unorm,
            1,
        );
        let bar_mapped = create_bar_columnar_mapped_pipeline(
            &device,
            &transform_bgl,
            &style_bgl,
            &style_map_bgl,
            wgpu::TextureFormat::Bgra8Unorm,
        );

        let _ = (mapped, picked, picked_mapped, errorbar_mapped, bar_mapped);
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
    }

    /// Wire up sampler + bind-group layout + bind group end-to-end; a
    /// slot-type mismatch would panic in `create_bind_group`.
    #[test]
    fn texture_sampler_bind_group_wires_up() {
        let Some((device, queue)) = shared_device() else {
            println!("no adapter — skipping bind group test");
            return;
        };

        let rgba: [u8; 16] = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
        ];
        let texture = upload_rgba_texture(
            &device,
            &queue,
            device.limits().max_texture_dimension_2d,
            2,
            2,
            &rgba,
        )
        .expect("upload texture");
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = create_linear_sampler(&device);
        let layout = create_texture_bind_group_layout(&device);
        let _bind_group = create_texture_bind_group(&device, &layout, &view, &sampler);

        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
    }

    /// Open a device + queue without a surface and print the adapter and a
    /// few limits (useful locally with `cargo test -- --nocapture`).
    /// Fails when no adapter is available.
    #[test]
    fn device_request_opens_device_and_queue() {
        let instance = create_instance();
        let adapter =
            request_adapter(&instance).expect("required GPU test: adapter initialization failed");
        let info = adapter.get_info();
        println!("adapter name    : {}", info.name);
        println!("adapter backend : {:?}", info.backend);
        println!("adapter type    : {:?}", info.device_type);
        println!("adapter driver  : {} / {}", info.driver, info.driver_info);
        match request_device(&adapter) {
            Ok((device, _queue)) => {
                let limits = device.limits();
                println!("device opened OK");
                println!(
                    "  max_texture_dim_2d     : {}",
                    limits.max_texture_dimension_2d
                );
                println!("  max_buffer_size        : {}", limits.max_buffer_size);
                println!("  max_bind_groups        : {}", limits.max_bind_groups);
            }
            Err(e) => panic!("request_device failed on available adapter: {e}"),
        }
    }
}
