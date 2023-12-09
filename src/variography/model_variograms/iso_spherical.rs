#[derive(Debug, Clone)]
pub struct IsoSpherical {
    pub range: f64,
    pub sill: f64,
}

impl IsoSpherical {
    pub fn new(range: f64, sill: f64) -> Self {
        Self { range, sill }
    }

    pub fn variogram(&self, h: f64) -> f64 {
        if h < self.range {
            return self.sill * (1.5 * h / self.range - 0.5 * (h / self.range).powi(3));
        }
        return self.sill;
    }

    pub fn covariogram(&self, h: f64) -> f64 {
        self.sill - self.variogram(h)
    }

    //derivative of variogram with respect to range
    pub fn variogram_dr(&self, h: f64) -> f64 {
        let r = self.range;

        return self.sill * (1.5 * h * h * h / (r * r * r * r) - 1.5 * h / (r * r));
    }

    //derivative of variogram with respect to sill
    //pub fn variogram_ds(&self, h: f32) -> f32 {
    //let r = self.range;

    //1.5 * h / (r) - 0.5 * h * h * h / (r * r * r)
    //}

    pub fn parameter_names() -> Vec<&'static str> {
        vec!["range"]
    }
}