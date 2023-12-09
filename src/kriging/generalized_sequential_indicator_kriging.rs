use crate::decomposition::lu::MiniLUSystem;
use crate::kriging::generalized_sequential_kriging::GSK;
use crate::spatial_database::MapConditioningProvider;
use crate::{
    geometry::ellipsoid::Ellipsoid, spatial_database::ConditioningProvider,
    variography::model_variograms::VariogramModel,
};

use itertools::izip;
use simba::simd::SimdPartialOrd;
use simba::simd::SimdRealField;
use simba::simd::SimdValue;

use super::generalized_sequential_kriging::GSKParameters;
use super::simple_kriging::ConditioningParams;
use super::simple_kriging::SKBuilder;
use super::simple_kriging::SupportInterface;
use super::simple_kriging::SupportTransform;

#[derive(Clone, Debug, Default)]
pub struct IKCPDF {
    pub p: Vec<f32>,
    pub x: Vec<f32>,
}

impl IKCPDF {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            p: Vec::with_capacity(capacity),
            x: Vec::with_capacity(capacity),
        }
    }

    pub fn correct(&mut self) {
        //clamp p within 0 and 1
        self.p.iter_mut().for_each(|x| {
            if *x > 1.0 {
                *x = 1.0;
            } else if *x < 0.0 {
                *x = 0.0;
            }
        });

        let mut curr_max = f32::MIN;
        let forward_running_max = self.p.iter().map(|v| {
            if v > &curr_max {
                curr_max = *v;
            }
            curr_max
        });

        let mut curr_min = f32::MAX;
        let backward_min = self.p.iter().rev().map(|v| {
            if v < &curr_min {
                curr_min = *v;
            }
            curr_min
        });

        self.p = izip!(forward_running_max, backward_min.rev())
            .map(|(f, b)| (f + b) / 2.0)
            .collect::<Vec<f32>>();

        //set p to 1.0 for last theshold
        *self.p.last_mut().unwrap() = 1.0;
    }
}

pub struct GSIK<S, V, VT>
where
    S: ConditioningProvider<Ellipsoid, f32, ConditioningParams> + Sync + std::marker::Send,
    V: VariogramModel<VT> + std::marker::Sync,
    VT: SimdPartialOrd + SimdRealField + SimdValue<Element = f32> + Copy,
{
    conditioning_data: S,
    variogram_models: Vec<V>,
    thresholds: Vec<f32>,
    search_ellipsoid: Ellipsoid,
    parameters: GSKParameters,
    phantom_v_type: std::marker::PhantomData<VT>,
}

impl<S, V, VT> GSIK<S, V, VT>
where
    S: ConditioningProvider<Ellipsoid, f32, ConditioningParams> + Sync + std::marker::Send,
    V: VariogramModel<VT> + std::marker::Sync,
    VT: SimdPartialOrd + SimdRealField + SimdValue<Element = f32> + Copy,
{
    pub fn new(
        conditioning_data: S,
        variogram_models: Vec<V>,
        thresholds: Vec<f32>,
        search_ellipsoid: Ellipsoid,
        parameters: GSKParameters,
    ) -> Self {
        Self {
            conditioning_data,
            variogram_models,
            thresholds,
            search_ellipsoid,
            parameters,
            phantom_v_type: std::marker::PhantomData,
        }
    }

    pub fn estimate<SKB, MS>(&mut self, groups: &Vec<Vec<SKB::Support>>) -> Vec<IKCPDF>
    where
        SKB: SKBuilder,
        S::Shape: SupportTransform<SKB::Support>,
        <SKB as SKBuilder>::Support: SupportInterface, // why do I need this the trait already requires this?!?!?
        SKB::Support: Sync,
        MS: MiniLUSystem,
        V: Clone,
    {
        let mut cpdfs: Vec<IKCPDF> = Vec::new();

        for (i, theshold) in self.thresholds.iter().enumerate() {
            //create indicator conditioning provider
            let cond = MapConditioningProvider::new(&mut self.conditioning_data, |x| {
                if *x <= *theshold {
                    *x = 1.0;
                } else {
                    *x = 0.0;
                }
            });

            //krig indicator data
            let gsk = GSK::new(
                cond,
                self.variogram_models[i].clone(),
                self.search_ellipsoid.clone(),
                self.parameters,
            );

            let estimates = gsk.estimate::<SKB, MS>(groups);

            //update cpdfs
            estimates.iter().enumerate().for_each(|(i, e)| {
                if let Some(cdpf) = cpdfs.get_mut(i) {
                    cdpf.p.push(*e);
                    cdpf.x.push(*theshold);
                } else {
                    let mut cpdf = IKCPDF::with_capacity(self.thresholds.len());
                    cpdf.p.push(*e);
                    cpdf.x.push(*theshold);
                    cpdfs.push(cpdf);
                }
            });
        }
        //cpdfs.iter_mut().for_each(|e| e.correct());
        cpdfs
    }
}

#[cfg(test)]
mod test {
    use std::{collections::HashMap, fs::File, io::Write};

    use nalgebra::{Point3, Translation3, UnitQuaternion, Vector3};
    use parry3d::bounding_volume::Aabb;
    use simba::simd::WideF32x8;

    use crate::{
        decomposition::lu::MiniLUOKSystem,
        kriging::{
            generalized_sequential_kriging::optimize_groups, simple_kriging::SKPointSupportBuilder,
        },
        spatial_database::{
            coordinate_system::CoordinateSystem, rtree_point_set::point_set::PointSet,
            DiscretiveVolume,
        },
        variography::model_variograms::spherical::SphericalVariogram,
    };

    use super::*;

    #[test]
    fn gsik_ok_test() {
        // create a gridded database from a csv file (walker lake)
        println!("Reading Cond Data");
        let cond = PointSet::from_csv_index("C:/Users/2jake/OneDrive - McGill University/Fall2022/MIME525/Project4/mineralized_domain_composites.csv", "X", "Y", "Z", "CU")
            .expect("Failed to create gdb");

        let thresholds = vec![
            0., 0.33333333, 0.66666667, 1., 1.33333333, 1.66666667, 2., 2.33333333, 2.66666667, 3.,
        ];

        //

        let vgram_rot = UnitQuaternion::identity();
        let range = Vector3::new(
            WideF32x8::splat(200.0),
            WideF32x8::splat(200.0),
            WideF32x8::splat(200.0),
        );
        let sill = WideF32x8::splat(1.0f32);

        let spherical_vgram = SphericalVariogram::new(range, sill, vgram_rot);

        // create search ellipsoid
        let search_ellipsoid = Ellipsoid::new(
            200f32,
            200f32,
            200f32,
            CoordinateSystem::new(Translation3::new(0.0, 0.0, 0.0), UnitQuaternion::identity()),
        );

        // create a gsk system
        let parameters = GSKParameters {
            max_group_size: 10,
            max_cond_data: 10,
            min_conditioned_octants: 1,
        };
        let mut gsk = GSIK::new(
            cond.clone(),
            vec![spherical_vgram; thresholds.len()],
            thresholds.clone(),
            search_ellipsoid,
            parameters,
        );

        println!("Reading Target Data");
        let targ = PointSet::<f32>::from_csv_index(
            "C:/Users/2jake/OneDrive - McGill University/Fall2022/MIME525/Project4/target.csv",
            "X",
            "Y",
            "Z",
            "V",
        )
        .unwrap();

        let points = targ.points.clone();

        //map points in vec of group of points (64)
        //map points in vec of group of points (64)
        let mut groups = Vec::new();
        //let mut group = Vec::new();
        for point in points.iter() {
            let aabb = Aabb::new(
                Point3::new(point.x, point.y, point.z),
                Point3::new(point.x + 5.0, point.y + 5.0, point.z + 10.0),
            );

            groups.push(aabb.discretize(5f32, 5f32, 10f32));
        }

        let time1 = std::time::Instant::now();
        let values = gsk.estimate::<SKPointSupportBuilder, MiniLUOKSystem>(&groups);
        let time2 = std::time::Instant::now();
        println!("Time: {:?}", (time2 - time1).as_secs());
        println!(
            "Points per minute: {}",
            values.len() as f32 / (time2 - time1).as_secs_f32() * 60.0
        );

        //save values to file for visualization

        let mut out = File::create("./test_results/lu_ik_ok.txt").unwrap();
        let _ = out.write_all(b"surfs\n");
        let _ = out.write_all(b"4\n");
        let _ = out.write_all(b"x\n");
        let _ = out.write_all(b"y\n");
        let _ = out.write_all(b"z\n");
        for theshold in thresholds.iter() {
            let _ = out.write_all(format!("leq_{}\n", theshold).as_bytes());
        }

        for (point, value) in points.iter().zip(values.iter()) {
            //println!("point: {:?}, value: {}", point, value);
            let _ = out.write_all(format!("{} {} {}", point.x, point.y, point.z).as_bytes());
            for v in value.p.iter() {
                let _ = out.write_all(format!(" {}", v).as_bytes());
            }
            let _ = out.write_all(b"\n");
        }

        let mut out = File::create("./test_results/lu_ik_ok.csv").unwrap();
        //write header
        let _ = out.write_all("X,Y,Z,DX,DY,DZ".as_bytes());
        for theshold in thresholds.iter() {
            let _ = out.write_all(format!(",leq_{}", theshold).as_bytes());
        }
        let _ = out.write_all(b"\n");

        //write each row

        for (point, value) in points.iter().zip(values.iter()) {
            //println!("point: {:?}, value: {}", point, value);
            let _ = out.write_all(
                format!("{},{},{},{},{},{}", point.x, point.y, point.z, 5, 5, 10).as_bytes(),
            );

            for v in value.p.iter() {
                let _ = out.write_all(format!(",{}", v).as_bytes());
            }
            let _ = out.write_all(b"\n");
        }
    }

    #[test]
    fn gsik_gc_zones() {
        // create a gridded database from a csv file (walker lake)
        println!("Reading Cond Data");
        // let cond = PointSet::from_csv_index("C:\\Users\\2jake\\OneDrive\\Desktop\\foresight\\whirlpool_geostats\\.dev-env\\config\\composited_drillholes.csv", "X", "Y", "Z", "LITH")
        //     .expect("Failed to create gdb");

        let mut reader =
            csv::Reader::from_path("C:\\Users\\2jake\\OneDrive\\Desktop\\foresight\\whirlpool_geostats\\.dev-env\\config\\composited_drillholes_reduced.csv").unwrap();
        //X	Y	Z	AU	Rock Unit	REGOLITH	LITH

        let mut points = Vec::new();
        let mut data = Vec::new();
        for record in reader.deserialize() {
            let record: (f32, f32, f32, f32, String, String, String) = record.unwrap();
            points.push(Point3::new(record.0, record.1, record.2));
            data.push(record.3);
        }

        // let mut ind_code = 0;
        // let mut lith_codes = HashMap::new();
        // let mut lith_codes_indexed = data.iter().for_each(|lith| {
        //     if !lith_codes.contains_key(lith) {
        //         lith_codes.insert(lith, ind_code);
        //         ind_code += 1;
        //     }
        // });

        // let ordered_lith = lith_codes
        //     .keys()
        //     .sorted()
        //     .map(|lith| lith.to_owned().to_owned())
        //     .collect::<Vec<_>>();

        // let mapped_data = data
        //     .iter()
        //     .map(|lith| lith_codes[lith] as f32)
        //     .collect::<Vec<f32>>();

        let cond = PointSet::new(points, data);

        let vgram_rot = UnitQuaternion::identity();
        let range = Vector3::new(
            WideF32x8::splat(10.0),
            WideF32x8::splat(20.0),
            WideF32x8::splat(10.0),
        );
        let sill = WideF32x8::splat(1.0f32);

        let spherical_vgram = SphericalVariogram::new(range, sill, vgram_rot);

        // create search ellipsoid
        let search_ellipsoid = Ellipsoid::new(
            50f32,
            50f32,
            50f32,
            CoordinateSystem::new(Translation3::new(0.0, 0.0, 0.0), UnitQuaternion::identity()),
        );

        println!("Reading Target Data");
        let mut reader =
            csv::Reader::from_path("C:\\GitRepos\\terrustrial\\data\\new_model.csv").unwrap();

        let mut aabbs = Vec::new();
        for record in reader.deserialize() {
            let record: (f32, f32, f32, f32, f32, f32, f32) = record.unwrap();
            aabbs.push(Aabb::new(
                Point3::new(record.0, record.1, record.2),
                Point3::new(
                    record.0 + record.3,
                    record.1 + record.4,
                    record.2 + record.5,
                ),
            ));
        }

        println!("Discretizing");
        //discretize each block
        let dx = 2f32;
        let dy = 2f32;
        let dz = 2f32;

        let mut block_inds = Vec::new();
        let points = aabbs
            .iter()
            .enumerate()
            .map(|(i, x)| {
                let disc_points = x.discretize(dx, dy, dz);
                block_inds.append(vec![i; disc_points.len()].as_mut());
                disc_points
            })
            .flatten()
            .collect::<Vec<_>>();

        println!("Optimizing groups");
        let (groups, point_inds) = optimize_groups(points.as_slice(), dx, dy, dz, 5, 5, 5);

        //gsk parameters
        let group_size = 125;
        let parameters = GSKParameters {
            max_group_size: group_size,
            max_cond_data: 20,
            min_conditioned_octants: 5,
        };
        let mut ind_value = HashMap::new();

        let mut lith_codes = HashMap::new();
        lith_codes.insert("0.5", 0.5);
        lith_codes.insert("0.3", 0.3);
        lith_codes.insert("0.0", 0.0);

        for (lith, lith_code) in lith_codes {
            let mut indicator_cond = cond.clone();
            indicator_cond.data_mut().iter_mut().for_each(|v| {
                if *v >= lith_code as f32 {
                    *v = 1.0
                } else {
                    *v = 0.0
                }
            });

            // create a gsk system
            let gsk = GSK::new(
                indicator_cond,
                spherical_vgram,
                search_ellipsoid.clone(),
                parameters,
            );

            let values = gsk.estimate::<SKPointSupportBuilder, MiniLUOKSystem>(&groups);

            let block_values = values.iter().zip(point_inds.iter().flatten()).fold(
                vec![vec![]; aabbs.len()],
                |mut acc, (value, ind)| {
                    acc[block_inds[*ind]].push(*value);
                    acc
                },
            );

            let avg_block_values = block_values
                .iter()
                .map(|x| {
                    x.iter()
                        .map(|v| if v.is_nan() { 0.0 } else { *v })
                        .sum::<f32>()
                        / x.len() as f32
                })
                .collect::<Vec<_>>();

            ind_value.insert(lith, avg_block_values);
        }

        //save values to file for visualization
        //let mut out = File::create("./test_results/gsk_large_model.csv").unwrap();
        let ordered_lith = vec!["0.0", "0.3", "0.5"];
        //write header
        let mut out_str = "X,Y,Z,DX,DY,DZ,".to_string();
        out_str += ordered_lith.join(",").as_str();
        out_str += "\n";

        for (i, aabb) in aabbs.iter().enumerate() {
            //println!("point: {:?}, value: {}", point, value);
            out_str += format!(
                "{},{},{},{},{},{}",
                aabb.mins.x,
                aabb.mins.y,
                aabb.mins.z,
                aabb.maxs.x - aabb.mins.x,
                aabb.maxs.y - aabb.mins.y,
                aabb.maxs.z - aabb.mins.z
            )
            .as_str();

            // let mut row_data = vec!["0"; ordered_lith.len()];
            // let max_ind = ordered_lith
            //     .iter()
            //     .map(|lith| ind_value[lith][i])
            //     .enumerate()
            //     .max_by(|(_, x), (_, y)| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal))
            //     .map(|(i, _)| i);
            for key in ordered_lith.iter() {
                out_str += format!(",{}", ind_value[key][i]).as_str();
            }
            // row_data[max_ind.unwrap()] = "1";
            // out_str += row_data.join(",").as_str();
            out_str += "\n";
        }
        let _ = std::fs::write("./test_results/gsik_gc_zones.csv", out_str);
        // let time1 = std::time::Instant::now();
        // let values = gsk
        //     .estimate::<SKPointSupportBuilder, NegativeFilteredMiniLUSystem<MiniLUOKSystem>>(
        //         &groups,
        //     );
        // let time2 = std::time::Instant::now();
        // println!("Time: {:?}", (time2 - time1).as_secs());
        // println!(
        //     "Points per minute: {}",
        //     values.len() as f32 / (time2 - time1).as_secs_f32() * 60.0
        // );

        //save values to file for visualization

        // let mut out = File::create("./test_results/lu_ik_ok.txt").unwrap();
        // let _ = out.write_all(b"surfs\n");
        // let _ = out.write_all(b"4\n");
        // let _ = out.write_all(b"x\n");
        // let _ = out.write_all(b"y\n");
        // let _ = out.write_all(b"z\n");
        // for theshold in thresholds.iter() {
        //     let _ = out.write_all(format!("leq_{}\n", theshold).as_bytes());
        // }

        // for (point, value) in points.iter().zip(values.iter()) {
        //     //println!("point: {:?}, value: {}", point, value);
        //     let _ = out.write_all(format!("{} {} {}", point.x, point.y, point.z).as_bytes());
        //     for v in value.p.iter() {
        //         let _ = out.write_all(format!(" {}", v).as_bytes());
        //     }
        //     let _ = out.write_all(b"\n");
        // }

        // let mut out = File::create("./test_results/lu_ik_ok.csv").unwrap();
        // //write header
        // let _ = out.write_all("X,Y,Z,DX,DY,DZ".as_bytes());
        // for theshold in thresholds.iter() {
        //     let _ = out.write_all(format!(",leq_{}", theshold).as_bytes());
        // }
        // let _ = out.write_all(b"\n");

        // //write each row

        // for (point, value) in points.iter().zip(values.iter()) {
        //     //println!("point: {:?}, value: {}", point, value);
        //     let _ = out.write_all(
        //         format!("{},{},{},{},{},{}", point.x, point.y, point.z, 5, 5, 10).as_bytes(),
        //     );

        //     for v in value.p.iter() {
        //         let _ = out.write_all(format!(",{}", v).as_bytes());
        //     }
        //     let _ = out.write_all(b"\n");
        // }
    }
}
