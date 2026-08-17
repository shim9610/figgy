use renderer::{ColumnPairWriter, ColumnSource, ColumnUploadStats, HiLoColumnSource};

struct DownstreamSource {
    values: [f64; 2],
}

impl ColumnSource for DownstreamSource {
    fn len(&self) -> usize {
        self.values.len()
    }

    fn min(&self) -> f64 {
        self.values[0]
    }

    fn max(&self) -> f64 {
        self.values[1]
    }

    fn write_f32_le_into(&self, dst: &mut [u8]) {
        for (bytes, &value) in dst.chunks_exact_mut(4).zip(&self.values) {
            bytes.copy_from_slice(&(value as f32).to_le_bytes());
        }
    }

    fn write_f32_pair_le_into_with_stats(
        &self,
        mut writer: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats {
        assert_eq!(writer.len(), self.values.len());
        for (index, &value) in self.values.iter().enumerate() {
            writer.write_pair(index, value as f32, 0.0);
        }
        ColumnUploadStats {
            min_positive: Some(self.values[0] as f32 as f64),
        }
    }
}

impl HiLoColumnSource for DownstreamSource {
    fn len(&self) -> usize {
        self.values.len()
    }

    fn min(&self) -> f64 {
        self.values[0]
    }

    fn max(&self) -> f64 {
        self.values[1]
    }

    fn write_f32_pair_le_into(&self, dst: &mut [u8]) {
        for (pair, &value) in dst.chunks_exact_mut(8).zip(&self.values) {
            pair[..4].copy_from_slice(&(value as f32).to_le_bytes());
            pair[4..].copy_from_slice(&0.0f32.to_le_bytes());
        }
    }

    fn write_f32_pair_le_into_with_stats(
        &self,
        mut writer: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats {
        assert_eq!(writer.len(), self.values.len());
        for (index, &value) in self.values.iter().enumerate() {
            writer.write_pair(index, value as f32, 0.0);
        }
        ColumnUploadStats {
            min_positive: Some(self.values[0] as f32 as f64),
        }
    }
}

fn scalar_len(source: &dyn ColumnSource) -> usize {
    source.len()
}

fn hilo_len(source: &dyn HiLoColumnSource) -> usize {
    source.len()
}

#[test]
fn custom_source_uses_only_renderer_public_types_and_remains_object_safe() {
    let source = DownstreamSource { values: [1.0, 2.0] };

    assert_eq!(scalar_len(&source), 2);
    assert_eq!(hilo_len(&source), 2);
}
