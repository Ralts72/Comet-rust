use super::*;
use comet_scene::{
    Animator, AudioSource, Character, CharacterAnimation, EntityRef, JointConnection, Model,
    NavigationAgent, NavigationRegion, PhysicsBody, Script, SkinBinding,
};
use std::collections::{BTreeMap, BTreeSet};

/// Optional author components. Identity, hierarchy, transform and links are always present.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ComponentKind {
    Model,
    Physics,
    Character,
    Script,
    PointJoint,
    HingeJoint,
    SliderJoint,
    NavigationRegion,
    NavigationAgent,
    Audio,
    Animation,
    Skin,
    CharacterAnimation,
}
impl ComponentKind {
    pub const ALL: &[Self] = &[
        Self::Model,
        Self::Physics,
        Self::Character,
        Self::Script,
        Self::PointJoint,
        Self::HingeJoint,
        Self::SliderJoint,
        Self::NavigationRegion,
        Self::NavigationAgent,
        Self::Audio,
        Self::Animation,
        Self::Skin,
        Self::CharacterAnimation,
    ];
    pub fn is_present(self, entity: &EntityData) -> bool {
        match self {
            Self::Model => entity.model.is_some(),
            Self::Physics => entity.physics.is_some(),
            Self::Character => entity.character.is_some(),
            Self::Script => entity.script.is_some(),
            Self::PointJoint => entity.point_joint.is_some(),
            Self::HingeJoint => entity.hinge_joint.is_some(),
            Self::SliderJoint => entity.slider_joint.is_some(),
            Self::NavigationRegion => entity.navigation_region.is_some(),
            Self::NavigationAgent => entity.navigation_agent.is_some(),
            Self::Audio => entity.audio_source.is_some(),
            Self::Animation => entity.animator.is_some(),
            Self::Skin => entity.skin.is_some(),
            Self::CharacterAnimation => entity.character_animation.is_some(),
        }
    }
    pub fn removal(self, entity: &EntityData) -> ComponentEdit {
        match self {
            Self::Model => ComponentEdit::Model(None),
            Self::Physics => ComponentEdit::Physics {
                body: None,
                character: None,
            },
            Self::Character => ComponentEdit::Physics {
                body: entity.physics.clone(),
                character: None,
            },
            Self::Script => ComponentEdit::Script(None),
            Self::PointJoint => ComponentEdit::Joint(JointEdit::Point(None)),
            Self::HingeJoint => ComponentEdit::Joint(JointEdit::Hinge(None)),
            Self::SliderJoint => ComponentEdit::Joint(JointEdit::Slider(None)),
            Self::NavigationRegion => ComponentEdit::Navigation {
                region: None,
                agent: entity.navigation_agent.clone(),
            },
            Self::NavigationAgent => ComponentEdit::Navigation {
                region: entity.navigation_region.clone(),
                agent: None,
            },
            Self::Audio => ComponentEdit::Audio(None),
            Self::Animation => ComponentEdit::Animation {
                animator: None,
                skin: entity.skin.clone(),
            },
            Self::Skin => ComponentEdit::Animation {
                animator: entity.animator.clone(),
                skin: None,
            },
            Self::CharacterAnimation => ComponentEdit::CharacterAnimation(None),
        }
    }
}

/// Component values only; entity identity and source ownership cannot be changed by this command.
pub enum ComponentEdit {
    Model(Option<Model>),
    Physics {
        body: Option<PhysicsBody>,
        character: Option<Character>,
    },
    Script(Option<Script>),
    Audio(Option<AudioSource>),
    Navigation {
        region: Option<NavigationRegion>,
        agent: Option<NavigationAgent>,
    },
    Animation {
        animator: Option<Animator>,
        skin: Option<SkinBinding>,
    },
    CharacterAnimation(Option<CharacterAnimation>),
    Joint(JointEdit),
}

impl Document {
    /// Validate the complete expanded candidate once, including dependency and reference cleanup.
    pub fn preview_components(
        &mut self,
        edits: impl IntoIterator<Item = (EntityId, ComponentEdit)>,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.preview_source(resolver, |next| {
            let indexes: BTreeMap<_, _> = next.base.entities.iter().enumerate().map(|(i, e)| (e.id, i)).collect();
            let mut seen = BTreeSet::new();
            let mut bodies = BTreeSet::new();
            let mut characters = BTreeSet::new();
            let mut animators = BTreeSet::new();
            for (id, edit) in edits {
                ensure!(seen.insert(id), "duplicate component target {id}");
                let index = *indexes.get(&id).context("component editing requires an authored entity")?;
                let entity = &mut next.base.entities[index];
                let before = (entity.physics.is_some(), entity.character.is_some(), entity.animator.is_some());
                match edit {
                    ComponentEdit::Model(value) => {
                        entity.model = value;
                        if entity.model.is_none() { entity.skin = None; }
                    }
                    ComponentEdit::Physics { body, character } => {
                        entity.physics = body;
                        entity.character = character;
                    }
                    ComponentEdit::Script(value) => entity.script = value,
                    ComponentEdit::Audio(value) => entity.audio_source = value,
                    ComponentEdit::Navigation { region, agent } => {
                        entity.navigation_region = region;
                        entity.navigation_agent = agent;
                    }
                    ComponentEdit::Animation { animator, skin } => {
                        entity.animator = animator;
                        entity.skin = skin;
                    }
                    ComponentEdit::CharacterAnimation(value) => entity.character_animation = value,
                    ComponentEdit::Joint(JointEdit::Point(value)) => entity.point_joint = value,
                    ComponentEdit::Joint(JointEdit::Hinge(value)) => entity.hinge_joint = value,
                    ComponentEdit::Joint(JointEdit::Slider(value)) => entity.slider_joint = value,
                }
                if before.0 && entity.physics.is_none() {
                    bodies.insert(id);
                    entity.point_joint = None;
                    entity.hinge_joint = None;
                    entity.slider_joint = None;
                }
                if before.1 && entity.character.is_none() {
                    characters.insert(id);
                    entity.navigation_agent = None;
                }
                if before.2 && entity.animator.is_none() { animators.insert(id); }
            }
            // Removing a component does not remove its entity or generic entity references.
            for entity in &mut next.base.entities {
                for settings in entity.point_joint.iter_mut().map(|j| &mut j.settings)
                    .chain(entity.hinge_joint.iter_mut().map(|j| &mut j.settings))
                    .chain(entity.slider_joint.iter_mut().map(|j| &mut j.settings)) {
                    if matches!(&settings.connection, JointConnection::Body(EntityRef::Local { entity }) if bodies.contains(entity)) {
                        settings.connection = JointConnection::Unbound;
                    }
                }
                if let Some(skin) = &mut entity.skin
                    && matches!(&skin.animator, Some(EntityRef::Local { entity }) if animators.contains(entity)) {
                    skin.animator = None;
                }
                if let Some(driver) = &mut entity.character_animation {
                    if matches!(&driver.character, Some(EntityRef::Local { entity }) if characters.contains(entity)) {
                        driver.character = None;
                    }
                    if matches!(&driver.animator, Some(EntityRef::Local { entity }) if animators.contains(entity)) {
                        driver.animator = None;
                    }
                }
            }
            Ok(())
        })
    }
    /// Body and controller form one candidate so intermediate invalid combinations never publish.
    pub fn preview_physics(
        &mut self,
        id: EntityId,
        body: Option<PhysicsBody>,
        character: Option<Character>,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.preview_components([(id, ComponentEdit::Physics { body, character })], resolver)
    }
    /// Model structure belongs to authored entities; asset and draw validation belongs to the host.
    pub fn preview_model(
        &mut self,
        id: EntityId,
        model: Option<Model>,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.preview_components([(id, ComponentEdit::Model(model))], resolver)
    }

    pub fn preview_script(
        &mut self,
        id: EntityId,
        script: Option<Script>,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.preview_components([(id, ComponentEdit::Script(script))], resolver)
    }

    pub fn preview_audio_source(
        &mut self,
        id: EntityId,
        audio: Option<comet_scene::AudioSource>,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.preview_components([(id, ComponentEdit::Audio(audio))], resolver)
    }

    /// Region and agent edits validate the entire candidate before replacing the accepted preview.
    pub fn preview_navigation(
        &mut self,
        id: EntityId,
        region: Option<NavigationRegion>,
        agent: Option<NavigationAgent>,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.preview_components(
            [(id, ComponentEdit::Navigation { region, agent })],
            resolver,
        )
    }

    pub fn preview_animation(
        &mut self,
        id: EntityId,
        animator: Option<comet_scene::Animator>,
        skin: Option<comet_scene::SkinBinding>,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.preview_components(
            [(id, ComponentEdit::Animation { animator, skin })],
            resolver,
        )
    }

    pub fn preview_character_animation(
        &mut self,
        id: EntityId,
        driver: Option<comet_scene::CharacterAnimation>,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.preview_components([(id, ComponentEdit::CharacterAnimation(driver))], resolver)
    }
}
