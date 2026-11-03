use super::*;

impl GraphicsState {
    pub(super) fn poll_gpu_timing(&mut self) {
        let Some(timing) = &mut self.gpu_timing else {
            return;
        };
        let _ = self.device.poll(wgpu::PollType::Poll);
        while let Ok((slot, mapped)) = timing.receiver.try_recv() {
            if mapped {
                if let Ok(bytes) = timing.readback_buffers[slot].slice(..).get_mapped_range() {
                    if bytes.len() >= 16 {
                        let start = u64::from_ne_bytes(bytes[0..8].try_into().unwrap());
                        let end = u64::from_ne_bytes(bytes[8..16].try_into().unwrap());
                        if end >= start {
                            let period_ns = self.queue.get_timestamp_period() as f64;
                            let elapsed_ms = (end - start) as f64 * period_ns / 1_000_000.0;
                            self.gpu_frame_time_ms = Some(elapsed_ms as f32);
                        }
                    }
                    drop(bytes);
                }
                timing.readback_buffers[slot].unmap();
            }
            timing.pending_slots[slot] = false;
        }
    }

    pub(super) fn refresh_frame_time_readout(&mut self) {
        if !self.frametime_mode {
            self.readout_vertex_count = 0;
            return;
        }
        let now = Instant::now();
        if self
            .readout_refreshed_at
            .is_some_and(|last| now.duration_since(last) < Duration::from_millis(200))
        {
            return;
        }
        self.readout_refreshed_at = Some(now);
        let gpu = match self.gpu_frame_time_ms {
            Some(milliseconds) => format!("GPU {:.1} ms", milliseconds),
            None if self.gpu_timing.is_some() => "GPU --.- ms".to_owned(),
            None => "GPU n/a".to_owned(),
        };
        let cpu = self.cpu_frame_time_ms.map_or_else(
            || "CPU work --.- ms".to_owned(),
            |ms| format!("CPU work {:.1} ms", ms),
        );
        let text = format!("{gpu}  |  {cpu}");
        if text == self.readout_text {
            return;
        }
        let vertices = match self.variable_layer.gpu_readout_vertices(&text) {
            Ok(vertices) => vertices,
            Err(error) => {
                eprintln!("failed to build GPU timing readout: {error}");
                return;
            }
        };
        let required_bytes = (vertices.len()
            * std::mem::size_of::<crate::graphics::pipelines::VariableVertex>())
            as u64;
        if required_bytes > self.readout_vertex_capacity {
            self.readout_vertex_capacity = required_bytes.next_power_of_two().max(4);
            self.readout_vertices = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("GPU frametime readout vertices"),
                size: self.readout_vertex_capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        self.readout_vertex_count = vertices.len().min(u32::MAX as usize) as u32;
        self.queue
            .write_buffer(&self.readout_vertices, 0, bytemuck::cast_slice(&vertices));
        self.readout_text = text;
    }
}
