//! `light.viewmodel`: what lights the first-person gun. Every lit lamp/light near the camera
//! (distance, illuminance at the camera in lux) and the viewmodel's materials (albedo,
//! roughness, metallic, reflectance, textures), for diagnosing a gun that clips to white.

use bevy::prelude::*;
use engine::devtools::{AddControlCommand, params};
use engine::gfx::MainCamera;
use serde_json::{Value, json};

pub(super) fn register(app: &mut App) {
    app.add_control_command("light.viewmodel", "Lights near the camera (lux at the camera) and the viewmodel's materials", params(&[("reflectance", "number", "dev A/B: set every viewmodel material's reflectance"), ("albedo", "number", "dev A/B: multiply every viewmodel albedo")]), report);
}

fn report(world: &mut World, p: Value) -> Result<Value, String> {
    let (cam_e, cam) = {
        let mut q = world.query_filtered::<(Entity, &GlobalTransform), (With<MainCamera>, With<Camera3d>)>();
        let (e, t) = q.iter(world).next().ok_or("no camera")?;
        (e, t.translation())
    };
    let mut lights = Vec::new();
    let mut pq = world.query::<(&GlobalTransform, &PointLight, Option<&Name>)>();
    for (t, l, n) in pq.iter(world) {
        let d = t.translation().distance(cam);
        if l.intensity > 0.0 && d < l.range {
            // Lumens → candela over the full sphere; illuminance at the camera (lux).
            lights.push(json!({"kind": "point", "name": n.map(|n| n.as_str().to_string()), "lm": l.intensity, "d": d, "lux": l.intensity / (4.0 * std::f32::consts::PI) / (d * d).max(1e-4), "radius": l.radius, "range": l.range}));
        }
    }
    let mut sq = world.query::<(&GlobalTransform, &SpotLight, Option<&Name>)>();
    for (t, l, n) in sq.iter(world) {
        let d = t.translation().distance(cam);
        if l.intensity > 0.0 && d < l.range {
            let to = (cam - t.translation()).normalize_or_zero();
            let inside = t.forward().dot(to) >= l.outer_angle.cos();
            lights.push(json!({"kind": "spot", "name": n.map(|n| n.as_str().to_string()), "lm": l.intensity, "d": d, "lux": l.intensity / (4.0 * std::f32::consts::PI) / (d * d).max(1e-4), "in_cone": inside, "radius": l.radius}));
        }
    }
    let mut dq = world.query::<&DirectionalLight>();
    let sun: Vec<f32> = dq.iter(world).map(|l| l.illuminance).collect();
    let exposure = world.get::<bevy::camera::Exposure>(cam_e).map(|e| e.ev100);
    // Viewmodel meshes: descendants of the main camera.
    let mut stack = vec![cam_e];
    let mut mats = std::collections::BTreeMap::new();
    while let Some(e) = stack.pop() {
        if let Some(c) = world.get::<Children>(e) {
            stack.extend(c.iter());
        }
        let Some(h) = world.get::<MeshMaterial3d<StandardMaterial>>(e) else { continue };
        let vis = world.get::<InheritedVisibility>(e).is_some_and(|v| v.get());
        let h = h.0.clone();
        if !mats.contains_key(&format!("{:?}", h.id())) {
            let mut assets = world.resource_mut::<Assets<StandardMaterial>>();
            if let Some(mut m) = assets.get_mut(&h) {
                if let Some(r) = p.get("reflectance").and_then(Value::as_f64) {
                    m.reflectance = r as f32;
                }
                if let Some(k) = p.get("albedo").and_then(Value::as_f64) {
                    let c = m.base_color.to_linear() * k as f32;
                    m.base_color = c.into();
                }
            }
        }
        let Some(m) = world.resource::<Assets<StandardMaterial>>().get(&h) else { continue };
        let c = m.base_color.to_linear();
        let name = world.get::<bevy::gltf::GltfMaterialName>(e).map(|n| n.0.clone()).unwrap_or_default();
        mats.entry(format!("{:?}", h.id())).or_insert_with(|| {
            json!({"name": name, "visible": vis, "base_linear": [c.red, c.green, c.blue], "metallic": m.metallic, "roughness": m.perceptual_roughness, "reflectance": m.reflectance,
                   "unlit": m.unlit, "emissive": [m.emissive.red, m.emissive.green, m.emissive.blue], "tex": [m.base_color_texture.is_some(), m.normal_map_texture.is_some(), m.metallic_roughness_texture.is_some()]})
        });
    }
    Ok(json!({"camera": [cam.x, cam.y, cam.z], "ev100": exposure, "sun_lux": sun, "lights": lights, "viewmodel_materials": mats.into_values().filter(|v| v["visible"] == true).collect::<Vec<_>>()}))
}
