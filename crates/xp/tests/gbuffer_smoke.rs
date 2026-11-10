//! Headless validation of the two ported passes.
//!
//! The `xp` package is a binary crate with no lib target, so the modules are pulled in by path
//! instead of `use xp::...` — these are the same files `main.rs` declares as modules, and they
//! refer to each other through `crate::gbuffer`, which resolves to the modules below.
//!
//! WGSL is compiled by wgpu at `create_shader_module`/`create_render_pipeline` time, not by
//! `cargo build`, so the real check is "does `PassChain::with_size` return Ok". It wraps all of
//! its eager creation in a validation error scope precisely so that a WGSL error arrives here as
//! an `Err`. The second half of the test records and submits one prepass draw and one sun-light
//! draw, which is what catches a bind group layout or vertex layout that does not match the
//! pipeline.

#[path = "../src/gbuffer.rs"]
mod gbuffer;
#[allow(dead_code)]
#[path = "../src/passes.rs"]
mod passes;

use anyhow::{Context, Result};
use bytemuck::Zeroable as _;

/// A 1x1 RGBA8 view to stand in for a material texture the install's content would otherwise
/// supply; the terrain bind group layout requires all five to be bound even when the draw only
/// reads one of them.
fn placeholder(device: &wgpu::Device, name: &str) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(name),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

#[test]
fn pass_chain_builds_and_draws_headless() -> Result<()> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        // No surface: this is a headless validation run, not a window.
        compatible_surface: None,
        ..Default::default()
    })) {
        Ok(adapter) => adapter,
        Err(e) => {
            eprintln!("SKIP pass_chain_builds_and_draws_headless: no adapter available ({e})");
            return Ok(());
        }
    };
    if !adapter
        .features()
        .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
        && adapter.get_info().backend == wgpu::Backend::Gl
    {
        eprintln!(
            "SKIP pass_chain_builds_and_draws_headless: GLES adapter without \
             TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES, which RGBA16Float attachments need"
        );
        return Ok(());
    }

    let (device, queue) =
        pollster::block_on(passes::request_device(&adapter, "headless gbuffer test"))
            .context("requesting a device")?;

    // The thing under test: three WGSL sources, two bind group layouts, two pipeline layouts and
    // two pipelines, all built eagerly. An Err here means a WGSL or format problem.
    let chain = passes::PassChain::with_size(&device, wgpu::TextureFormat::Rgba8Unorm, 64, 64)
        .context("PassChain::with_size(64x64, Rgba8Unorm)")?;

    assert_eq!(
        chain.gbuffer_color_views().len(),
        gbuffer::GBUFFER_FORMATS.len(),
        "the g-buffer must have five color attachments"
    );
    assert_eq!(chain.size(), (64, 64));

    let base = placeholder(&device, "tex_base");
    let comp = placeholder(&device, "tex_comp");
    let comp2 = placeholder(&device, "tex_comp2");
    let noise = placeholder(&device, "tex_noise");
    let bump = placeholder(&device, "tex_bump");
    let textures = gbuffer::TerrainTextures {
        base: &base,
        comp: &comp,
        comp2: &comp2,
        noise: &noise,
        bump: &bump,
    };
    let terrain_bg = chain.terrain_bind_group(&device, &textures);

    let hdr = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hdr scene (test)"),
        size: wgpu::Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let hdr_view = hdr.create_view(&wgpu::TextureViewDescriptor::default());

    let clear_color = wgpu::LoadOp::Clear(wgpu::Color::BLACK);
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    {
        let mut enc = device.create_command_encoder(&Default::default());

        // Pass 1: the g-buffer MRT. Five color attachments plus the depth attachment, exactly the
        // attachment set PassChain allocates.
        {
            let attachments: Vec<Option<wgpu::RenderPassColorAttachment<'_>>> = chain
                .gbuffer_color_views()
                .iter()
                .map(|view| {
                    Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: clear_color,
                            store: wgpu::StoreOp::Store,
                        },
                    })
                })
                .collect();
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("test prepass"),
                color_attachments: &attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: chain.depth_view(),
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let uniforms = gbuffer::TerrainUniforms::zeroed();
            // The triangle-list variant; the line-list variant of the same program is the other
            // arm of `PassChain::terrain_pipeline_for`.
            chain.draw_prepass(
                &queue,
                &mut pass,
                &terrain_bg,
                &uniforms,
                xp_core::scene::Topology::Triangles,
            );
        }

        // Pass 2: the full-screen sun pass, with no depth attachment (the pipeline has none).
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("test sun light"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &hdr_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: clear_color,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let uniforms = gbuffer::SunUniforms::zeroed();
            chain.draw_sun_light(&queue, &mut pass, &uniforms);
        }

        queue.submit([enc.finish()]);
    }
    if let Some(e) = pollster::block_on(scope.pop()) {
        return Err(anyhow::anyhow!(
            "recording the two ported passes failed: {e}"
        ));
    }

    Ok(())
}

/// The uniform blocks must stay byte-identical to the WGSL structs they feed; the layout pins in
/// `gbuffer.rs` are compile-time, so this only checks the sizes the bind groups advertise.
#[test]
fn uniform_blocks_match_the_wgsl_layout() {
    use bytemuck::Pod;
    assert_eq!(gbuffer::TERRAIN_UNIFORM_SIZE, 336);
    assert_eq!(gbuffer::SUN_UNIFORM_SIZE, 304);
    assert_eq!(std::mem::align_of::<gbuffer::TerrainUniforms>(), 16);
    assert_eq!(std::mem::align_of::<gbuffer::SunUniforms>(), 16);
    // Pod is unsafe to implement; asserting it here keeps the derive honest in the test too.
    fn assert_pod<T: Pod>() {}
    assert_pod::<gbuffer::TerrainUniforms>();
    assert_pod::<gbuffer::SunUniforms>();
    assert_pod::<gbuffer::SunLight>();
}
