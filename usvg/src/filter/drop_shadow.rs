// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use strict_num::PositiveF64;
use svgtypes::Length;

use super::{Input, Kind, Primitive};
use crate::svgtree::{self, AId};
use crate::{converter, Color, Opacity, SvgColorExt};

/// A drop shadow filter primitive.
///
/// This is essentially `feGaussianBlur`, `feOffset` and `feFlood` joined together.
///
/// `feDropShadow` element in the SVG.
#[derive(Clone, Debug)]
pub struct DropShadow {
    /// Identifies input for the given filter primitive.
    ///
    /// `in` in the SVG.
    pub input: Input,

    /// The amount to offset the input graphic along the X-axis.
    pub dx: f64,

    /// The amount to offset the input graphic along the Y-axis.
    pub dy: f64,

    /// A standard deviation along the X-axis.
    ///
    /// `stdDeviation` in the SVG.
    pub std_dev_x: PositiveF64,

    /// A standard deviation along the Y-axis.
    ///
    /// `stdDeviation` in the SVG.
    pub std_dev_y: PositiveF64,

    /// A flood color.
    ///
    /// `flood-color` in the SVG.
    pub color: Color,

    /// A flood opacity.
    ///
    /// `flood-opacity` in the SVG.
    pub opacity: Opacity,
}

pub(crate) fn convert(
    fe: svgtree::Node,
    primitives: &[Primitive],
    state: &converter::State,
) -> Kind {
    let (std_dev_x, std_dev_y) = super::gaussian_blur::convert_std_dev_attr(fe, "2 2");

    let (color, opacity) = fe
        .attribute(AId::FloodColor)
        .unwrap_or_else(svgtypes::Color::black)
        .split_alpha();

    Kind::DropShadow(DropShadow {
        input: super::resolve_input(fe, AId::In, primitives),
        dx: fe.convert_user_length(AId::Dx, state, Length::new_number(2.0)),
        dy: fe.convert_user_length(AId::Dy, state, Length::new_number(2.0)),
        std_dev_x,
        std_dev_y,
        color,
        opacity: opacity * fe.attribute(AId::FloodOpacity).unwrap_or(Opacity::ONE),
    })
}
