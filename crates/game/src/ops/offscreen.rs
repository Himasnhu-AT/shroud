//! `--offscreen` (test tooling): the window stays hidden and the player camera renders into
//! an image, so screenshots work even when the window can't be seen (another app fullscreen,
//! other Space, display asleep).

use bevy::prelude::*;
use bevy::render::render_resource::{TextureFormat, TextureUsages};

use crate::player::PlayerCamera;
use crate::state::has_visuals;

pub const SIZE: (u32, u32) = (1920, 1080);

pub fn enabled() -> bool {
    super::has_flag("offscreen")
}

/// The image the player camera renders into.
#[derive(Resource, Clone)]
pub struct OffscreenTarget(pub Handle<Image>);

pub struct OffscreenPlugin;

impl Plugin for OffscreenPlugin {
    fn build(&self, app: &mut App) {
        if enabled() {
            app.add_systems(PostStartup, retarget_camera.run_if(has_visuals));
        }
    }
}

fn retarget_camera(mut commands: Commands, mut images: ResMut<Assets<Image>>, cams: Query<Entity, With<PlayerCamera>>) {
    let mut image = Image::new_target_texture(SIZE.0, SIZE.1, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let handle = images.add(image);
    for cam in &cams {
        commands
            .entity(cam)
            .insert((bevy::camera::RenderTarget::from(handle.clone()), bevy::ui::IsDefaultUiCamera));
    }
    commands.insert_resource(OffscreenTarget(handle));
    info!("offscreen: rendering the player camera to a {}x{} image", SIZE.0, SIZE.1);
}
