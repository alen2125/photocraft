//! The numbers VaM's body parts run on, for models that do not carry their own (a VRM): masses
//! and drag by body part, and the strength of the muscle holding each joint at its rest pose.
//! A model made from VaM's own figure ships these in its file and never gets here.

use photocraft_scene3d::model3d::DriveData;

/// A body part's make-up.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Defaults {
    pub mass: f32,
    pub drag: f32,
    pub angular_drag: f32,
    pub muscle: DriveData,
}

const fn d(mass: f32, drag: f32, angular_drag: f32, spring: f32, damper: f32, max_force: f32) -> Defaults {
    Defaults { mass, drag, angular_drag, muscle: DriveData { spring, damper, max_force } }
}

/// By VRM humanoid bone name; `None` for bones that have no body of their own.
pub(crate) fn defaults(humanoid: &str) -> Option<Defaults> {
    Some(match humanoid {
        "hips" => d(4.0, 1.0, 10.0, 0.0, 0.0, 0.0),
        "spine" => d(2.0, 1.0, 10.0, 100.0, 1.0, 500.0),
        "chest" | "upperChest" => d(2.0, 1.0, 10.0, 100.0, 1.0, 500.0),
        "neck" => d(0.7, 1.0, 10.0, 20.0, 0.5, 5.0),
        "head" => d(1.0, 1.0, 10.0, 20.0, 0.5, 5.0),
        "leftShoulder" | "rightShoulder" => d(0.3, 1.0, 1.0, 20.0, 0.5, 5.0),
        "leftUpperArm" | "rightUpperArm" => d(0.6, 0.1, 10.0, 20.0, 0.5, 5.0),
        "leftLowerArm" | "rightLowerArm" => d(0.3, 0.1, 10.0, 25.0, 0.5, 5.0),
        "leftHand" | "rightHand" => d(1.0, 0.1, 10.0, 40.0, 0.5, 20.0),
        "leftUpperLeg" | "rightUpperLeg" => d(2.0, 1.0, 10.0, 50.0, 0.5, 18.0),
        "leftLowerLeg" | "rightLowerLeg" => d(1.0, 0.1, 10.0, 20.0, 0.5, 5.0),
        "leftFoot" | "rightFoot" => d(1.0, 1.0, 10.0, 20.0, 0.5, 5.0),
        "leftToes" | "rightToes" => d(0.1, 0.0, 0.05, 20.0, 0.5, 5.0),
        _ => return None,
    })
}
