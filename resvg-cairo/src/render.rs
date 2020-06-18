// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use log::warn;

pub(crate) mod prelude {
    pub(crate) use usvg::{
        TransformFromBBox, FuzzyEq, FuzzyZero, NodeExt, IsDefault, FitTo,
        Size, ScreenSize, Rect, ScreenRect,
    };
    pub(crate) use crate::layers::Layers;
    pub(crate) use crate::Options;
    pub(crate) use super::*;
}

use prelude::*;


/// Indicates the current rendering state.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum RenderState {
    /// A default value. Doesn't indicate anything.
    Ok,
    /// Indicates that the current rendering task should stop after reaching the specified node.
    RenderUntil(usvg::Node),
    /// Indicates that `usvg::FilterInput::BackgroundImage` rendering task was finished.
    BackgroundFinished,
}


pub trait ConvTransform<T> {
    fn to_native(&self) -> T;
    fn from_native(_: &T) -> Self;
}

impl ConvTransform<cairo::Matrix> for usvg::Transform {
    fn to_native(&self) -> cairo::Matrix {
        cairo::Matrix::new(self.a, self.b, self.c, self.d, self.e, self.f)
    }

    fn from_native(ts: &cairo::Matrix) -> Self {
        Self::new(ts.xx, ts.yx, ts.xy, ts.yy, ts.x0, ts.y0)
    }
}


pub trait ReCairoContextExt {
    fn set_source_color(&self, color: usvg::Color, opacity: usvg::Opacity);
    fn reset_source_rgba(&self);
}

impl ReCairoContextExt for cairo::Context {
    fn set_source_color(&self, color: usvg::Color, opacity: usvg::Opacity) {
        self.set_source_rgba(
            color.red as f64 / 255.0,
            color.green as f64 / 255.0,
            color.blue as f64 / 255.0,
            opacity.value(),
        );
    }

    fn reset_source_rgba(&self) {
        self.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    }
}


pub(crate) fn render_node_to_canvas(
    node: &usvg::Node,
    opt: &Options,
    view_box: usvg::ViewBox,
    img_size: ScreenSize,
    state: &mut RenderState,
    cr: &cairo::Context,
) {
    let mut layers = Layers::new(img_size);

    apply_viewbox_transform(view_box, img_size, &cr);

    let curr_ts = cr.get_matrix();
    let mut ts = node.abs_transform();
    ts.append(&node.transform());

    cr.transform(ts.to_native());
    render_node(node, opt, state, &mut layers, cr);
    cr.set_matrix(curr_ts);
}

pub fn create_surface(
    size: ScreenSize,
    opt: &Options,
) -> Option<(cairo::ImageSurface, ScreenSize)> {
    let img_size = opt.fit_to.fit_to(size)?;
    let surface = create_subsurface(img_size)?;
    Some((surface, img_size))
}

pub fn create_subsurface(size: ScreenSize) -> Option<cairo::ImageSurface> {
    let surface = cairo::ImageSurface::create(
        cairo::Format::ARgb32,
        size.width() as i32,
        size.height() as i32,
    );

    match surface {
        Ok(surface) => Some(surface),
        Err(_) => {
            warn!("Failed to create a {}x{} surface.", size.width(), size.height());
            None
        }
    }
}

/// Applies viewbox transformation to the painter.
fn apply_viewbox_transform(
    view_box: usvg::ViewBox,
    img_size: ScreenSize,
    cr: &cairo::Context,
) {
    let ts = usvg::utils::view_box_to_transform(view_box.rect, view_box.aspect, img_size.to_size());
    cr.transform(ts.to_native());
}

pub(crate) fn render_node(
    node: &usvg::Node,
    opt: &Options,
    state: &mut RenderState,
    layers: &mut Layers,
    cr: &cairo::Context,
) -> Option<Rect> {
    match *node.borrow() {
        usvg::NodeKind::Svg(_) => {
            render_group(node, opt, state, layers, cr)
        }
        usvg::NodeKind::Path(ref path) => {
            crate::path::draw(&node.tree(), path, opt, cr)
        }
        usvg::NodeKind::Image(ref img) => {
            Some(crate::image::draw(img, opt, cr))
        }
        usvg::NodeKind::Group(ref g) => {
            render_group_impl(node, g, opt, state, layers, cr)
        }
        _ => None,
    }
}

pub(crate) fn render_group(
    parent: &usvg::Node,
    opt: &Options,
    state: &mut RenderState,
    layers: &mut Layers,
    cr: &cairo::Context,
) -> Option<Rect> {
    let curr_ts = cr.get_matrix();
    let mut g_bbox = Rect::new_bbox();

    for node in parent.children() {
        match state {
            RenderState::Ok => {}
            RenderState::RenderUntil(ref last) => {
                if node == *last {
                    // Stop rendering.
                    *state = RenderState::BackgroundFinished;
                    break;
                }
            }
            RenderState::BackgroundFinished => break,
        }

        cr.transform(node.transform().to_native());

        let bbox = render_node(&node, opt, state, layers, cr);
        if let Some(bbox) = bbox {
            if let Some(bbox) = bbox.transform(&node.transform()) {
                g_bbox = g_bbox.expand(bbox);
            }
        }

        // Revert transform.
        cr.set_matrix(curr_ts);
    }

    // Check that bbox was changed, otherwise we will have a rect with x/y set to f64::MAX.
    if g_bbox.fuzzy_ne(&Rect::new_bbox()) {
        Some(g_bbox)
    } else {
        None
    }
}

fn render_group_impl(
    node: &usvg::Node,
    g: &usvg::Group,
    opt: &Options,
    state: &mut RenderState,
    layers: &mut Layers,
    cr: &cairo::Context,
) -> Option<Rect> {
    let sub_surface = layers.get()?;
    let mut sub_surface = sub_surface.borrow_mut();

    let curr_ts = cr.get_matrix();

    let bbox = {
        let sub_cr = cairo::Context::new(&*sub_surface);
        sub_cr.set_matrix(curr_ts);

        render_group(node, opt, state, layers, &sub_cr)
    };

    // During the background rendering for filters,
    // an opacity, a filter, a clip and a mask should be ignored for the inner group.
    // So we are simply rendering the `sub_img` without any postprocessing.
    //
    // SVG spec, 15.6 Accessing the background image
    // 'Any filter effects, masking and group opacity that might be set on A[i] do not apply
    // when rendering the children of A[i] into BUF[i].'
    if *state == RenderState::BackgroundFinished {
        let curr_matrix = cr.get_matrix();
        cr.set_matrix(cairo::Matrix::identity());
        cr.set_source_surface(&*sub_surface, 0.0, 0.0);
        cr.paint();
        cr.set_matrix(curr_matrix);
        cr.reset_source_rgba();
        return bbox;
    }

    // Filter can be rendered on an object without a bbox,
    // as long as filter uses `userSpaceOnUse`.
    if let Some(ref id) = g.filter {
        if let Some(filter_node) = node.tree().defs_by_id(id) {
            if let usvg::NodeKind::Filter(ref filter) = *filter_node.borrow() {
                let ts = usvg::Transform::from_native(&curr_ts);
                let background = prepare_filter_background(node, filter, opt);
                let fill_paint = prepare_filter_fill_paint(node, filter, bbox, ts, opt, &sub_surface);
                let stroke_paint = prepare_filter_stroke_paint(node, filter, bbox, ts, opt, &sub_surface);
                crate::filter::apply(filter, bbox, &ts, opt, &node.tree(),
                                     background.as_ref(), fill_paint.as_ref(), stroke_paint.as_ref(),
                                     &mut *sub_surface);
            }
        }
    }

    // Clipping and masking can be done only for objects with a valid bbox.
    if let Some(bbox) = bbox {
        if let Some(ref id) = g.clip_path {
            if let Some(clip_node) = node.tree().defs_by_id(id) {
                if let usvg::NodeKind::ClipPath(ref cp) = *clip_node.borrow() {
                    let sub_cr = cairo::Context::new(&*sub_surface);
                    sub_cr.set_matrix(curr_ts);

                    crate::clip::clip(&clip_node, cp, opt, bbox, layers, &sub_cr);
                }
            }
        }

        if let Some(ref id) = g.mask {
            if let Some(mask_node) = node.tree().defs_by_id(id) {
                if let usvg::NodeKind::Mask(ref mask) = *mask_node.borrow() {
                    let sub_cr = cairo::Context::new(&*sub_surface);
                    sub_cr.set_matrix(curr_ts);

                    crate::mask::mask(&mask_node, mask, opt, bbox, layers, &sub_cr);
                }
            }
        }
    }

    let curr_matrix = cr.get_matrix();
    cr.set_matrix(cairo::Matrix::identity());
    cr.set_source_surface(&*sub_surface, 0.0, 0.0);
    if !g.opacity.is_default() {
        cr.paint_with_alpha(g.opacity.value());
    } else {
        cr.paint();
    }

    cr.set_matrix(curr_matrix);

    // All layers must be unlinked from the main context/cr after used.
    // TODO: find a way to automate this
    cr.reset_source_rgba();

    bbox
}

/// Renders an image used by `BackgroundImage` or `BackgroundAlpha` filter inputs.
fn prepare_filter_background(
    parent: &usvg::Node,
    filter: &usvg::Filter,
    opt: &Options,
) -> Option<cairo::ImageSurface> {
    let start_node = parent.filter_background_start_node(filter)?;

    let tree = parent.tree();
    let (surface, img_size) = create_surface(tree.svg_node().size.to_screen_size(), opt)?;
    let view_box = tree.svg_node().view_box;

    let cr = cairo::Context::new(&surface);
    // Render from the `start_node` until the `parent`. The `parent` itself is excluded.
    let mut state = RenderState::RenderUntil(parent.clone());
    render_node_to_canvas(&start_node, opt, view_box, img_size, &mut state, &cr);

    Some(surface)
}

/// Renders an image used by `FillPaint`/`StrokePaint` filter input.
///
/// FillPaint/StrokePaint is mostly an undefined behavior and will produce different results
/// in every application.
/// And since there are no expected behaviour, we will simply fill the filter region.
///
/// https://github.com/w3c/fxtf-drafts/issues/323
fn prepare_filter_fill_paint(
    parent: &usvg::Node,
    filter: &usvg::Filter,
    bbox: Option<Rect>,
    ts: usvg::Transform,
    opt: &Options,
    canvas: &cairo::ImageSurface,
) -> Option<cairo::ImageSurface> {
    let region = crate::filter::calc_region(filter, bbox, &ts, canvas).ok()?;
    let surface = create_subsurface(region.size())?;
    if let usvg::NodeKind::Group(ref g) = *parent.borrow() {
        if let Some(paint) = g.filter_fill.clone() {
            let cr = cairo::Context::new(&*surface);
            let style_bbox = bbox.unwrap_or_else(|| Rect::new(0.0, 0.0, 1.0, 1.0).unwrap());
            let fill = Some(usvg::Fill::from_paint(paint));
            crate::paint_server::fill(&parent.tree(), &fill, opt, style_bbox, &cr);
            cr.rectangle(0.0, 0.0, region.width() as f64, region.height() as f64);
            cr.paint();
        }
    }

    Some(surface)
}

/// The same as `prepare_filter_fill_paint`, but for `StrokePaint`.
fn prepare_filter_stroke_paint(
    parent: &usvg::Node,
    filter: &usvg::Filter,
    bbox: Option<Rect>,
    ts: usvg::Transform,
    opt: &Options,
    canvas: &cairo::ImageSurface,
) -> Option<cairo::ImageSurface> {
    let region = crate::filter::calc_region(filter, bbox, &ts, canvas).ok()?;
    let surface = create_subsurface(region.size())?;
    if let usvg::NodeKind::Group(ref g) = *parent.borrow() {
        if let Some(paint) = g.filter_stroke.clone() {
            let cr = cairo::Context::new(&*surface);
            let style_bbox = bbox.unwrap_or_else(|| Rect::new(0.0, 0.0, 1.0, 1.0).unwrap());
            let fill = Some(usvg::Fill::from_paint(paint));
            crate::paint_server::fill(&parent.tree(), &fill, opt, style_bbox, &cr);
            cr.rectangle(0.0, 0.0, region.width() as f64, region.height() as f64);
            cr.paint();
        }
    }

    Some(surface)
}
