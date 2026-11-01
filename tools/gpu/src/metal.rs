//! 原生调用和 unsafe 仅存在于此适配原型。所有 CPU 读取都在完成反馈之后。
use anyhow::{Context, Result, ensure};
use objc2::{
    rc::{Retained, autoreleasepool},
    runtime::ProtocolObject,
};
use objc2_foundation::{NSArray, NSString, NSURL};
use objc2_metal::*;
use serde_json::{Value, json};
use std::{ffi::c_void, ptr::NonNull};
type Device = ProtocolObject<dyn MTLDevice>;
type Buffer = ProtocolObject<dyn MTLBuffer>;
type Command = ProtocolObject<dyn MTLCommandBuffer>;
fn completed(c: &Command) -> Result<()> {
    c.commit();
    c.waitUntilCompleted();
    ensure!(
        c.status() == MTLCommandBufferStatus::Completed,
        "GPU command failed: {:?}",
        c.error()
    );
    Ok(())
}
fn buffer(d: &Device, n: usize) -> Result<Retained<Buffer>> {
    d.newBufferWithLength_options(n, MTLResourceOptions::StorageModeShared)
        .context("buffer allocation")
}
fn library(d: &Device, source: &str) -> Result<Retained<ProtocolObject<dyn MTLLibrary>>> {
    d.newLibraryWithSource_options_error(&NSString::from_str(source), None)
        .map_err(|e| anyhow::anyhow!("Metal compile: {e}"))
}
fn pipeline(
    d: &Device,
    l: &ProtocolObject<dyn MTLLibrary>,
    name: &str,
) -> Result<Retained<ProtocolObject<dyn MTLComputePipelineState>>> {
    let f = l
        .newFunctionWithName(&NSString::from_str(name))
        .context("missing entry point")?;
    d.newComputePipelineStateWithFunction_error(&f)
        .map_err(|e| anyhow::anyhow!("pipeline: {e}"))
}
fn dispatch(
    c: &Command,
    p: &ProtocolObject<dyn MTLComputePipelineState>,
    b: &Buffer,
    n: usize,
) -> Result<()> {
    let e = c.computeCommandEncoder().context("compute encoder")?;
    e.setComputePipelineState(p);
    // Buffer has n f32 elements and is retained until the command is completed.
    unsafe {
        e.setBuffer_offset_atIndex(Some(b), 0, 0);
    }
    e.dispatchThreads_threadsPerThreadgroup(
        MTLSize {
            width: n,
            height: 1,
            depth: 1,
        },
        MTLSize {
            width: 64,
            height: 1,
            depth: 1,
        },
    );
    e.endEncoding();
    Ok(())
}
fn interval(c: &Command) -> [f64; 2] {
    [c.GPUStartTime(), c.GPUEndTime()]
}
fn queues(d: &Device, p: &ProtocolObject<dyn MTLComputePipelineState>) -> Result<Value> {
    let q1 = d.newCommandQueue().context("queue 1")?;
    let q2 = d.newCommandQueue().context("queue 2")?;
    let n = 262144;
    let a = buffer(d, n * 4)?;
    let b = buffer(d, n * 4)?;
    let event = d.newEvent().context("cross-queue event")?;
    let ca = q1.commandBuffer().context("command A")?;
    let cb = q2.commandBuffer().context("command B")?;
    ca.setLabel(Some(&NSString::from_str("Independent compute A")));
    cb.setLabel(Some(&NSString::from_str("Independent compute B")));
    dispatch(&ca, p, &a, n)?;
    ca.encodeSignalEvent_value(&event, 1);
    dispatch(&cb, p, &b, n)?;
    // Both independent workloads are enqueued before waiting. The consumer has a real event dependency.
    ca.commit();
    cb.commit();
    let consumer = q2.commandBuffer().context("consumer")?;
    consumer.encodeWaitForEvent_value(&event, 1);
    let readback = buffer(d, 4)?;
    let blit = consumer.blitCommandEncoder().context("blit")?;
    unsafe {
        blit.copyFromBuffer_sourceOffset_toBuffer_destinationOffset_size(&a, 0, &readback, 0, 4);
    }
    blit.endEncoding();
    completed(&consumer)?;
    ca.waitUntilCompleted();
    cb.waitUntilCompleted();
    ensure!(
        ca.status() == MTLCommandBufferStatus::Completed
            && cb.status() == MTLCommandBufferStatus::Completed,
        "queue workload failed"
    );
    let va = unsafe { *(readback.contents().as_ptr().cast::<f32>()) };
    let vb = unsafe { *(b.contents().as_ptr().cast::<f32>()) };
    ensure!(
        va.is_finite() && va > 0.1 && (va - vb).abs() < 1e-5,
        "cross-queue data mismatch"
    );
    let ia = interval(&ca);
    let ib = interval(&cb);
    Ok(
        json!({"status":"passed","queue_count":2,"elements_per_queue":n,"iterations":512,"a_seconds":ia,"b_seconds":ib,
        "command_interval_overlap_ms":(ia[1].min(ib[1])-ia[0].max(ib[0])).max(0.0)*1000.0,
        "event_dependency_readback":va,"note":"Command-buffer timestamp overlap includes scheduling; it is not a claimed frame speedup."}),
    )
}
fn alias(d: &Device, p: &ProtocolObject<dyn MTLComputePipelineState>) -> Result<Value> {
    let n = 4096;
    let bytes = n * 4;
    let opts = MTLResourceOptions::StorageModePrivate;
    let sa = d.heapBufferSizeAndAlignWithLength_options(bytes, opts);
    let desc = MTLHeapDescriptor::new();
    desc.setType(MTLHeapType::Placement);
    desc.setStorageMode(MTLStorageMode::Private);
    desc.setSize(sa.size.next_multiple_of(sa.align));
    let heap = d.newHeapWithDescriptor(&desc).context("placement heap")?;
    // Same offset is reused only after GPU use of the previous resource has completed.
    let a =
        unsafe { heap.newBufferWithLength_options_offset(bytes, opts, 0) }.context("alias A")?;
    let q = d.newCommandQueue().context("alias queue")?;
    let ca = q.commandBuffer().context("alias command")?;
    dispatch(&ca, p, &a, n)?;
    completed(&ca)?;
    let offset_a = a.heapOffset();
    unsafe {
        a.makeAliasable();
    }
    let b =
        unsafe { heap.newBufferWithLength_options_offset(bytes, opts, 0) }.context("alias B")?;
    let cb = q.commandBuffer().context("alias command B")?;
    dispatch(&cb, p, &b, n)?;
    let out = buffer(d, bytes)?;
    let blit = cb.blitCommandEncoder().context("alias readback")?;
    unsafe {
        blit.copyFromBuffer_sourceOffset_toBuffer_destinationOffset_size(&b, 0, &out, 0, bytes);
    }
    blit.endEncoding();
    completed(&cb)?;
    let value = unsafe { *(out.contents().as_ptr().cast::<f32>()) };
    ensure!(value > 0.1 && value.is_finite(), "alias result");
    ensure!(offset_a == b.heapOffset(), "allocation not aliased");
    Ok(
        json!({"status":"passed","heap_bytes":heap.size(),"logical_resource_bytes":bytes*2,"a_offset":offset_a,"b_offset":b.heapOffset(),"allocation_bytes":sa.size,"readback":value}),
    )
}
fn raytrace(d: &Device, l: &ProtocolObject<dyn MTLLibrary>) -> Result<Value> {
    ensure!(
        d.supportsRaytracing(),
        "device does not support Metal ray tracing"
    );
    let verts: [f32; 9] = [-1., -1., 0., 1., -1., 0., 0., 1., 0.];
    let vb = unsafe {
        d.newBufferWithBytes_length_options(
            NonNull::new(verts.as_ptr() as *mut c_void).unwrap(),
            36,
            MTLResourceOptions::StorageModeShared,
        )
    }
    .context("triangle buffer")?;
    let geometry = MTLAccelerationStructureTriangleGeometryDescriptor::new();
    geometry.setVertexBuffer(Some(&vb));
    geometry.setVertexStride(12);
    geometry.setTriangleCount(1);
    geometry.setOpaque(true);
    let desc = MTLPrimitiveAccelerationStructureDescriptor::new();
    desc.setUsage(MTLAccelerationStructureUsage::Refit);
    let base: &MTLAccelerationStructureGeometryDescriptor = &geometry;
    desc.setGeometryDescriptors(Some(&NSArray::from_slice(&[base])));
    let sizes = d.accelerationStructureSizesWithDescriptor(&desc);
    let structure = d
        .newAccelerationStructureWithSize(sizes.accelerationStructureSize)
        .context("acceleration structure")?;
    let scratch = buffer(
        d,
        sizes
            .buildScratchBufferSize
            .max(sizes.refitScratchBufferSize),
    )?;
    let q = d.newCommandQueue().context("ray queue")?;
    let build = q.commandBuffer().context("build command")?;
    let encoder = build
        .accelerationStructureCommandEncoder()
        .context("AS encoder")?;
    encoder.buildAccelerationStructure_descriptor_scratchBuffer_scratchBufferOffset(
        &structure, &desc, &scratch, 0,
    );
    encoder.endEncoding();
    completed(&build)?;
    let p = pipeline(d, l, "trace_triangle")?;
    let trace = |expected: f32| -> Result<[f32; 2]> {
        let result = buffer(d, 8)?;
        let c = q.commandBuffer().context("trace command")?;
        let e = c.computeCommandEncoder().context("ray encoder")?;
        e.setComputePipelineState(&p);
        unsafe {
            e.setAccelerationStructure_atBufferIndex(Some(&structure), 0);
            e.setBuffer_offset_atIndex(Some(&result), 0, 1);
        }
        e.dispatchThreads_threadsPerThreadgroup(
            MTLSize {
                width: 2,
                height: 1,
                depth: 1,
            },
            MTLSize {
                width: 2,
                height: 1,
                depth: 1,
            },
        );
        e.endEncoding();
        completed(&c)?;
        let v = unsafe { std::slice::from_raw_parts(result.contents().as_ptr().cast::<f32>(), 2) };
        ensure!(
            (v[0] - expected).abs() < 1e-4 && v[1] == -1.0,
            "ray hit/miss mismatch: {v:?}"
        );
        Ok([v[0], v[1]])
    };
    let before = trace(2.)?;
    // Modify geometry after the first trace has completed, then refit and trace again.
    unsafe {
        for i in [2, 5, 8] {
            *vb.contents().as_ptr().cast::<f32>().add(i) = 0.5;
        }
    }
    let c = q.commandBuffer().context("refit command")?;
    let e = c
        .accelerationStructureCommandEncoder()
        .context("refit encoder")?;
    unsafe {
        e.refitAccelerationStructure_descriptor_destination_scratchBuffer_scratchBufferOffset(
            &structure,
            &desc,
            None,
            Some(&scratch),
            0,
        );
    }
    e.endEncoding();
    completed(&c)?;
    let after = trace(1.5)?;
    Ok(
        json!({"status":"passed","supports_raytracing":true,"before_hit_miss":before,"after_refit_hit_miss":after,"build_gpu_ms":(build.GPUEndTime()-build.GPUStartTime())*1000.0,"structure_bytes":sizes.accelerationStructureSize}),
    )
}
struct Capture {
    manager: Retained<MTLCaptureManager>,
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.manager.stopCapture();
    }
}
fn capture(d: &Device, output: &std::path::Path) -> Result<Capture> {
    let manager = unsafe { MTLCaptureManager::sharedCaptureManager() };
    ensure!(
        manager.supportsDestination(MTLCaptureDestination::GPUTraceDocument),
        "GPU capture unavailable; launch with MTL_CAPTURE_ENABLED=1"
    );
    let path = output.join("metal.gputrace");
    ensure!(
        !path.exists(),
        "GPU trace already exists; choose a new output directory"
    );
    let desc = MTLCaptureDescriptor::new();
    unsafe {
        desc.setCaptureObject(Some(d.as_ref()));
    }
    desc.setDestination(MTLCaptureDestination::GPUTraceDocument);
    desc.setOutputURL(Some(&NSURL::fileURLWithPath(&NSString::from_str(
        &path.to_string_lossy(),
    ))));
    manager
        .startCaptureWithDescriptor_error(&desc)
        .map_err(|e| anyhow::anyhow!("GPU capture: {e}"))?;
    Ok(Capture { manager })
}
pub fn run(output: &std::path::Path, capture_gpu: bool) -> Result<Value> {
    autoreleasepool(|_| {
        let d = MTLCreateSystemDefaultDevice().context("Metal unavailable")?;
        let _capture = if capture_gpu {
            Some(capture(&d, output)?)
        } else {
            None
        };
        let l = library(&d, include_str!("../shaders/capabilities.metal"))?;
        let p = pipeline(&d, &l, "workload")?;
        Ok(json!({"backend":"Metal", "device":d.name().to_string(),
            "multi_queue":queues(&d,&p)?, "native_alias":alias(&d,&p)?,
            "hardware_raytrace":raytrace(&d,&l)?}))
    })
}
