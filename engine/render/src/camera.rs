use anyhow::{Result, ensure};
use comet_core::math::{Mat4, Vec3};
const HALF_FOV: f64 = std::f64::consts::FRAC_PI_8;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Projection {
    #[default]
    Perspective,
    Orthographic {
        height: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub eye: Vec3,
    pub target: Vec3,
    pub projection: Projection,
    pub near: f32,
    pub far: f32,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            eye: Vec3::new(5., 3.5, 7.),
            target: Vec3::new(0., 0.7, 0.),
            projection: Projection::Perspective,
            near: 0.05,
            far: 200.,
        }
    }
}
impl Camera {
    pub fn validate(self) -> Result<()> {
        let direction = (self.eye.as_dvec3() - self.target.as_dvec3()).try_normalize();
        ensure!(
            self.eye.is_finite() && self.target.is_finite() && direction.is_some(),
            "invalid view camera"
        );
        ensure!(
            direction.unwrap().y.abs() < 0.9999,
            "view camera is parallel to its up axis"
        );
        ensure!(
            self.near.is_finite() && self.far.is_finite() && self.near > 0. && self.far > self.near,
            "invalid camera clip range"
        );
        if let Projection::Orthographic { height } = self.projection {
            ensure!(
                height.is_finite() && height > 0.,
                "invalid orthographic height"
            );
        }
        let projection = self.view_projection(1.);
        ensure!(
            projection.is_finite() && projection.inverse().is_finite(),
            "camera projection exceeds arithmetic range"
        );
        Ok(())
    }

    pub fn view_projection(self, aspect: f32) -> Mat4 {
        let aspect = aspect.max(0.01);
        let projection = match self.projection {
            Projection::Perspective => {
                comet_core::math::perspective((HALF_FOV * 2.) as f32, aspect, self.near, self.far)
            }
            Projection::Orthographic { height } => {
                let y = height * 0.5;
                let x = y * aspect;
                comet_core::math::orthographic(-x, x, -y, y, self.near, self.far)
            }
        };
        projection * comet_core::math::look_at(self.eye, self.target, Vec3::Y)
    }

    /// Visible world height at the target, shared by navigation and projection switching.
    pub fn visible_height(self) -> f32 {
        match self.projection {
            Projection::Perspective => {
                (2. * self.eye.as_dvec3().distance(self.target.as_dvec3()) * HALF_FOV.tan()) as f32
            }
            Projection::Orthographic { height } => height,
        }
    }

    /// Map viewport UV to the plane through the target, perpendicular to the view.
    pub fn focus_plane_point(self, uv: [f32; 2], aspect: f32) -> Option<Vec3> {
        if self.validate().is_err()
            || !aspect.is_finite()
            || aspect <= 0.
            || !uv
                .into_iter()
                .all(|v| v.is_finite() && (0. ..=1.).contains(&v))
        {
            return None;
        }
        let offset = self.target.as_dvec3() - self.eye.as_dvec3();
        let distance = offset.length();
        if distance < f64::from(self.near) || distance > f64::from(self.far) {
            return None;
        }
        let forward = offset.normalize();
        let right = forward.cross(comet_core::math::DVec3::Y).normalize();
        let up = right.cross(forward);
        let height = match self.projection {
            Projection::Perspective => 2. * distance * HALF_FOV.tan(),
            Projection::Orthographic { height } => f64::from(height),
        };
        let point = self.target.as_dvec3()
            + right * ((f64::from(uv[0]) - 0.5) * height * f64::from(aspect.max(0.01)))
            + up * ((0.5 - f64::from(uv[1])) * height);
        let result = point.as_vec3();
        result.is_finite().then_some(result)
    }

    /// Return a fully checked view before the caller replaces its accepted camera.
    pub fn fit_bounds(self, [min, max]: [Vec3; 2], aspect: f32) -> Result<Self> {
        self.validate()?;
        ensure!(
            min.is_finite()
                && max.is_finite()
                && min.cmple(max).all()
                && aspect.is_finite()
                && aspect > 0.,
            "invalid focus bounds or viewport"
        );
        let center = (min.as_dvec3() + max.as_dvec3()) * 0.5;
        let radius = (max.as_dvec3() - min.as_dvec3()).length() * 0.5;
        let radius = if radius == 0. { 0.1 } else { radius };
        let offset = self.eye.as_dvec3() - self.target.as_dvec3();
        let mut next = self;
        let distance = match self.projection {
            Projection::Perspective => {
                let vertical = HALF_FOV;
                let horizontal = (vertical.tan() * f64::from(aspect.max(0.01))).atan();
                radius * 1.2 / vertical.min(horizontal).sin()
            }
            Projection::Orthographic { .. } => {
                next.projection = Projection::Orthographic {
                    height: (2. * radius * 1.2 / f64::from(aspect.clamp(0.01, 1.))) as f32,
                };
                offset.length().max(radius * 2. + f64::from(self.near))
            }
        };
        next.target = center.as_vec3();
        next.eye = (center + offset.normalize() * distance).as_vec3();
        next.near = f64::from(self.near).min((distance - radius * 1.5).max(distance * 1e-4)) as f32;
        next.far = f64::from(self.far).max(distance + radius * 1.5) as f32;
        next.validate()?;
        // Casting back to GPU precision must still fit all corners, including the depth range.
        let projection = next.view_projection(aspect);
        for x in [min.x, max.x] {
            for y in [min.y, max.y] {
                for z in [min.z, max.z] {
                    let p = projection * Vec3::new(x, y, z).extend(1.);
                    ensure!(
                        p.is_finite()
                            && p.w > 0.
                            && p.z >= 0.
                            && p.z <= p.w
                            && p.x.abs() <= p.w
                            && p.y.abs() <= p.w,
                        "focus bounds cannot be represented at camera precision"
                    );
                }
            }
        }
        Ok(next)
    }

    pub fn light_projection() -> Mat4 {
        comet_core::math::orthographic(-7., 7., -7., 7., 0.1, 30.)
            * comet_core::math::look_at(Vec3::new(6., 10., 5.), Vec3::ZERO, Vec3::Y)
    }
}

#[cfg(test)]
mod tests;
