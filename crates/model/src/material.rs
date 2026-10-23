//! The surface material of a bar, a box or a sector.
//!
//! One enum serves every bounded chart family, and a name has one look: a
//! bar, a box and a flat (zero depth, zero tilt) pie top drawn with the same
//! material and parameters are shaded by the same function. A bar or a box is
//! a flat body: its normal is constant, so the looks that only respond to a
//! changing normal (`Toon`, `Pearl`) are rejected there, see
//! [`Material::supported_on_flat_body`].

/// Procedural surface material. The variant order is the GPU material kind,
/// so a name has one number in every family.
///
/// Not `#[non_exhaustive]`: an exhaustive `match` stops compiling when a
/// material is added, which is the intended upgrade signal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Material {
    /// The fill color, unlit. Ignores every material parameter.
    #[default]
    Flat,
    Matte,
    Ceramic,
    BrushedMetal,
    Paper,
    Wood,
    SatinMetal,
    Toon,
    Enamel,
    Hatch,
    Pearl,
}

impl Material {
    /// Whether the material has a visible look on a flat body (a bar or a
    /// box). `Toon` quantizes and `Pearl` tints by how the normal turns away
    /// from the light and the viewer; on a flat body the normal is constant,
    /// so both would draw as `Flat` with a uniform shift. Sectors accept
    /// every material.
    pub fn supported_on_flat_body(self) -> bool {
        !matches!(self, Material::Toon | Material::Pearl)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Material; 11] = [
        Material::Flat,
        Material::Matte,
        Material::Ceramic,
        Material::BrushedMetal,
        Material::Paper,
        Material::Wood,
        Material::SatinMetal,
        Material::Toon,
        Material::Enamel,
        Material::Hatch,
        Material::Pearl,
    ];

    #[test]
    fn flat_bodies_support_all_but_toon_and_pearl() {
        let unsupported: Vec<Material> = ALL
            .into_iter()
            .filter(|m| !m.supported_on_flat_body())
            .collect();
        assert_eq!(unsupported, [Material::Toon, Material::Pearl]);
    }

    #[test]
    fn all_lists_every_variant_in_kind_order() {
        for (kind, material) in ALL.into_iter().enumerate() {
            assert_eq!(material as usize, kind);
        }
        assert_eq!(Material::default(), Material::Flat);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn variants_serialize_as_their_names() {
        for material in ALL {
            let text = serde_json::to_string(&material).unwrap();
            assert_eq!(text, format!("\"{material:?}\""));
            assert_eq!(serde_json::from_str::<Material>(&text).unwrap(), material);
        }
    }
}
