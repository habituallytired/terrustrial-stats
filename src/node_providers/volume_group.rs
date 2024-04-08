use nalgebra::{Point3, UnitQuaternion};

use super::NodeProvider;

/// A node provider for a group of volumes.
/// Suitable for block-point, block-block, and point-block kriging.
pub struct VolumeGroupProvider {
    pub volumes: Vec<Vec<Point3<f32>>>,
    pub group_inds: Vec<usize>,
    pub orientations: Vec<UnitQuaternion<f32>>,
}

impl VolumeGroupProvider {
    pub fn get_group(&self, group: usize) -> &[Vec<Point3<f32>>] {
        let start = self.group_inds[group];
        let end = if group == self.group_inds.len() - 1 {
            self.volumes.len()
        } else {
            self.group_inds[group + 1]
        };

        &self.volumes[start..end]
    }

    pub fn from_groups(
        volumes: Vec<Vec<Vec<Point3<f32>>>>,
        orientations: Vec<UnitQuaternion<f32>>,
    ) -> Self {
        let mut group_inds = Vec::new();
        let mut volumes_flat = Vec::new();

        for volume in volumes {
            let start = volumes_flat.len();
            volumes_flat.extend(volume);
            group_inds.push(start);
        }

        Self {
            volumes: volumes_flat,
            group_inds,
            orientations,
        }
    }
}

impl NodeProvider for VolumeGroupProvider {
    type Support = Vec<Point3<f32>>;

    #[inline(always)]
    fn n_groups(&self) -> usize {
        self.group_inds.len()
    }

    #[inline(always)]
    fn get_group(&self, group: usize) -> &[Self::Support] {
        self.get_group(group)
    }

    #[inline(always)]
    fn get_orientation(&self, group: usize) -> &UnitQuaternion<f32> {
        &self.orientations[group]
    }

    // fn groups_and_orientations(
    //     &self,
    // ) -> impl ParallelIterator<Item = (&[Self::Support], UnitQuaternion<f32>)> {
    //     (0..self.group_inds.len())
    //         .into_par_iter()
    //         .map(|group| (self.get_group(group), self.orientations[group]))
    // }
}
