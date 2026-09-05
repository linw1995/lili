use lili_core::{PetLifecycleState, PetNotificationKind};
use serde::{Deserialize, Serialize};

use crate::AnimationState;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PreviewScene {
    Idle,
    Running,
    Review,
    Attention,
    Failed,
    Waiting,
    Click,
}

impl PreviewScene {
    pub const fn spec(self) -> PreviewSceneSpec {
        match self {
            Self::Idle => PreviewSceneSpec {
                scene: Self::Idle,
                lifecycle: PetLifecycleState::Idle,
                animation: AnimationState::Idle,
                notification: None,
            },
            Self::Running => PreviewSceneSpec {
                scene: Self::Running,
                lifecycle: PetLifecycleState::ActivityReminder,
                animation: AnimationState::Running,
                notification: None,
            },
            Self::Review => PreviewSceneSpec {
                scene: Self::Review,
                lifecycle: PetLifecycleState::Review,
                animation: AnimationState::Review,
                notification: Some(PetNotificationKind::Completion),
            },
            Self::Attention => PreviewSceneSpec {
                scene: Self::Attention,
                lifecycle: PetLifecycleState::Waiting,
                animation: AnimationState::Waiting,
                notification: Some(PetNotificationKind::Attention),
            },
            Self::Failed => PreviewSceneSpec {
                scene: Self::Failed,
                lifecycle: PetLifecycleState::Failed,
                animation: AnimationState::Failed,
                notification: Some(PetNotificationKind::Failure),
            },
            Self::Waiting => PreviewSceneSpec {
                scene: Self::Waiting,
                lifecycle: PetLifecycleState::Waiting,
                animation: AnimationState::Waiting,
                notification: Some(PetNotificationKind::Attention),
            },
            Self::Click => PreviewSceneSpec {
                scene: Self::Click,
                lifecycle: PetLifecycleState::Idle,
                animation: AnimationState::Waving,
                notification: None,
            },
        }
    }

    pub const fn all() -> &'static [Self; 7] {
        &PREVIEW_SCENES
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreviewSceneSpec {
    scene: PreviewScene,
    lifecycle: PetLifecycleState,
    animation: AnimationState,
    notification: Option<PetNotificationKind>,
}

impl PreviewSceneSpec {
    pub const fn scene(self) -> PreviewScene {
        self.scene
    }

    pub const fn lifecycle(self) -> PetLifecycleState {
        self.lifecycle
    }

    pub const fn animation(self) -> AnimationState {
        self.animation
    }

    pub const fn notification(self) -> Option<PetNotificationKind> {
        self.notification
    }
}

pub const PREVIEW_SCENES: [PreviewScene; 7] = [
    PreviewScene::Idle,
    PreviewScene::Running,
    PreviewScene::Review,
    PreviewScene::Attention,
    PreviewScene::Failed,
    PreviewScene::Waiting,
    PreviewScene::Click,
];

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn preview_scene_order_and_tokens_are_stable() {
        assert_eq!(
            PreviewScene::all(),
            &[
                PreviewScene::Idle,
                PreviewScene::Running,
                PreviewScene::Review,
                PreviewScene::Attention,
                PreviewScene::Failed,
                PreviewScene::Waiting,
                PreviewScene::Click,
            ]
        );
        let encoded = serde_json::to_value(PreviewScene::Click).unwrap();
        assert_eq!(encoded, json!("click"));
    }

    #[test]
    fn each_preview_scene_has_one_pure_runtime_mapping() {
        let expected = [
            (
                PreviewScene::Idle,
                PetLifecycleState::Idle,
                AnimationState::Idle,
                None,
            ),
            (
                PreviewScene::Running,
                PetLifecycleState::ActivityReminder,
                AnimationState::Running,
                None,
            ),
            (
                PreviewScene::Review,
                PetLifecycleState::Review,
                AnimationState::Review,
                Some(PetNotificationKind::Completion),
            ),
            (
                PreviewScene::Attention,
                PetLifecycleState::Waiting,
                AnimationState::Waiting,
                Some(PetNotificationKind::Attention),
            ),
            (
                PreviewScene::Failed,
                PetLifecycleState::Failed,
                AnimationState::Failed,
                Some(PetNotificationKind::Failure),
            ),
            (
                PreviewScene::Waiting,
                PetLifecycleState::Waiting,
                AnimationState::Waiting,
                Some(PetNotificationKind::Attention),
            ),
            (
                PreviewScene::Click,
                PetLifecycleState::Idle,
                AnimationState::Waving,
                None,
            ),
        ];

        for (scene, lifecycle, animation, notification) in expected {
            let spec = scene.spec();
            assert_eq!(spec.scene(), scene);
            assert_eq!(spec.lifecycle(), lifecycle);
            assert_eq!(spec.animation(), animation);
            assert_eq!(spec.notification(), notification);
        }
    }

    #[test]
    fn preview_scene_deserialization_rejects_unknown_tokens() {
        assert!(serde_json::from_value::<PreviewScene>(json!("unknown")).is_err());
    }
}
