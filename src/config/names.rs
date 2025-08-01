//! Implementation for [`Names`](`crate::config::Names`). Defines default name
//! templates and handles resolving templates into strings.

use crate::config;
use crate::state;

pub use crate::config::{name_template::NameTemplate, tag::Tag};
use crate::config::{
    tag::{ClientTag, DeviceTag, NodeTag},
    Names,
};

impl Names {
    pub fn default_stream() -> Vec<NameTemplate> {
        vec!["{node:node.name}: {node:media.name}".parse().unwrap()]
    }

    pub fn default_endpoint() -> Vec<NameTemplate> {
        vec![
            "{device:device.nick}".parse().unwrap(),
            "{node:node.description}".parse().unwrap(),
        ]
    }

    pub fn default_device() -> Vec<NameTemplate> {
        vec![
            "{device:device.nick}".parse().unwrap(),
            "{device:device.description}".parse().unwrap(),
        ]
    }

    /// Tries to resolve an object's name.
    ///
    /// Returns a name using the first template string that can be successfully
    /// resolved using the resolver.
    ///
    /// Precedence is:
    ///
    /// 1. Overrides
    /// 2. Stream/endpoint/device default templates
    /// 3. Fallback
    pub fn resolve<T: TagResolver + NameResolver>(
        &self,
        state: &state::State,
        resolver: &T,
    ) -> Option<String> {
        resolver
            .templates(state, self)
            .iter()
            .find_map(|template| {
                template.render(|tag| resolver.resolve_tag(state, *tag))
            })
            .or(resolver.fallback().cloned())
    }
}

impl Default for Names {
    fn default() -> Self {
        Self {
            stream: Self::default_stream(),
            endpoint: Self::default_endpoint(),
            device: Self::default_device(),
            overrides: Vec::new(),
        }
    }
}

pub trait TagResolver {
    fn resolve_tag<'a>(
        &'a self,
        state: &'a state::State,
        tag: Tag,
    ) -> Option<&'a String>;
}

pub trait NameResolver: TagResolver {
    fn fallback(&self) -> Option<&String>;

    fn templates<'a>(
        &self,
        state: &state::State,
        names: &'a config::Names,
    ) -> &'a Vec<NameTemplate>;

    fn name_override<'a>(
        &self,
        state: &state::State,
        overrides: &'a [config::NameOverride],
        override_type: config::OverrideType,
    ) -> Option<&'a Vec<NameTemplate>> {
        overrides.iter().find_map(|name_override| {
            (name_override.types.contains(&override_type)
                && self.resolve_tag(state, name_override.property)
                    == Some(&name_override.value))
            .then_some(&name_override.templates)
        })
    }
}

impl TagResolver for state::Device {
    /// Resolve a tag using Device.
    fn resolve_tag<'a>(
        &'a self,
        _state: &'a state::State,
        tag: Tag,
    ) -> Option<&'a String> {
        match tag {
            Tag::Device(DeviceTag::DeviceName) => self.name.as_ref(),
            Tag::Device(DeviceTag::DeviceNick) => self.nick.as_ref(),
            Tag::Device(DeviceTag::DeviceDescription) => {
                self.description.as_ref()
            }
            Tag::Node(_) => None,
            Tag::Client(_) => None,
        }
    }
}

impl NameResolver for state::Device {
    fn fallback(&self) -> Option<&String> {
        self.name.as_ref()
    }

    fn templates<'a>(
        &self,
        state: &state::State,
        names: &'a config::Names,
    ) -> &'a Vec<NameTemplate> {
        self.name_override(
            state,
            &names.overrides,
            config::OverrideType::Device,
        )
        .unwrap_or(&names.device)
    }
}

impl TagResolver for state::Node {
    /// Resolve a tag using Node. Falls back on resolving using the linked
    /// Device, if present.
    fn resolve_tag<'a>(
        &'a self,
        state: &'a state::State,
        tag: Tag,
    ) -> Option<&'a String> {
        match tag {
            Tag::Node(NodeTag::NodeName) => self.name.as_ref(),
            Tag::Node(NodeTag::NodeNick) => self.nick.as_ref(),
            Tag::Node(NodeTag::NodeDescription) => self.description.as_ref(),
            Tag::Node(NodeTag::MediaName) => self.media_name.as_ref(),
            Tag::Device(_) => {
                let device = state.devices.get(&self.device_id?)?;
                device.resolve_tag(state, tag)
            }
            Tag::Client(_) => {
                let client = state.clients.get(&self.client_id?)?;
                client.resolve_tag(state, tag)
            }
        }
    }
}

impl NameResolver for state::Node {
    fn fallback(&self) -> Option<&String> {
        self.name.as_ref()
    }

    fn templates<'a>(
        &self,
        state: &state::State,
        names: &'a config::Names,
    ) -> &'a Vec<NameTemplate> {
        match self.media_class.as_ref() {
            Some(media_class)
                if media_class.is_sink() || media_class.is_source() =>
            {
                self.name_override(
                    state,
                    &names.overrides,
                    config::OverrideType::Endpoint,
                )
                .unwrap_or(&names.endpoint)
            }
            _ => self
                .name_override(
                    state,
                    &names.overrides,
                    config::OverrideType::Stream,
                )
                .unwrap_or(&names.stream),
        }
    }
}

impl TagResolver for state::Client {
    /// Resolve a tag using Client.
    fn resolve_tag<'a>(
        &'a self,
        _state: &'a state::State,
        tag: Tag,
    ) -> Option<&'a String> {
        match tag {
            Tag::Client(ClientTag::ApplicationName) => {
                self.application_name.as_ref()
            }
            Tag::Client(ClientTag::ApplicationProcessBinary) => {
                self.application_process_binary.as_ref()
            }
            Tag::Node(_) => None,
            Tag::Device(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture_manager::CaptureManager;
    use crate::config::{NameOverride, Names, OverrideType};
    use crate::event::MonitorEvent;
    use crate::media_class::MediaClass;
    use crate::object::ObjectId;
    use crate::state::State;

    #[test]
    fn default_stream() {
        // Just make sure this doesn't panic.
        let _ = Names::default_stream();
    }

    #[test]
    fn default_endpoint() {
        // Just make sure this doesn't panic.
        let _ = Names::default_endpoint();
    }

    #[test]
    fn default_device() {
        // Just make sure this doesn't panic.
        let _ = Names::default_device();
    }

    struct Fixture {
        state: State,
        capture_manager: CaptureManager,
        device_id: ObjectId,
        node_id: ObjectId,
        client_id: ObjectId,
    }

    impl Default for Fixture {
        fn default() -> Self {
            let mut state = State::default();
            let mut capture_manager = CaptureManager::default();

            let device_id = ObjectId::from_raw_id(0);
            let node_id = ObjectId::from_raw_id(1);
            let client_id = ObjectId::from_raw_id(2);

            let events = vec![
                MonitorEvent::DeviceName(
                    device_id,
                    String::from("Device name"),
                ),
                MonitorEvent::DeviceNick(
                    device_id,
                    String::from("Device nick"),
                ),
                MonitorEvent::NodeName(node_id, String::from("Node name")),
                MonitorEvent::NodeNick(node_id, String::from("Node nick")),
                MonitorEvent::ClientApplicationName(
                    client_id,
                    String::from("Client name"),
                ),
            ];

            for event in events {
                state.update(&mut capture_manager, event);
            }

            Self {
                state,
                capture_manager,
                device_id,
                node_id,
                client_id,
            }
        }
    }

    #[test]
    fn render_endpoint() {
        let mut fixture = Fixture::default();

        fixture.state.update(
            &mut fixture.capture_manager,
            MonitorEvent::NodeMediaClass(
                fixture.node_id,
                MediaClass::from("Audio/Sink"),
            ),
        );
        let names = Names {
            endpoint: vec!["{node:node.nick}".parse().unwrap()],
            ..Default::default()
        };

        let node = fixture.state.nodes.get(&fixture.node_id).unwrap();
        let result = names.resolve(&fixture.state, node);
        assert_eq!(result, Some(String::from("Node nick")))
    }

    #[test]
    fn render_endpoint_missing_tag() {
        let mut fixture = Fixture::default();

        fixture.state.update(
            &mut fixture.capture_manager,
            MonitorEvent::NodeMediaClass(
                fixture.node_id,
                MediaClass::from("Audio/Sink"),
            ),
        );

        let names = Names {
            endpoint: vec!["{node:node.description}".parse().unwrap()],
            ..Default::default()
        };

        let node = fixture.state.nodes.get(&fixture.node_id).unwrap();
        let result = names.resolve(&fixture.state, node);
        // Should fall back to node name
        assert_eq!(result, Some(String::from("Node name")))
    }

    #[test]
    fn render_device_missing_tag() {
        let fixture = Fixture::default();

        let names = Names {
            device: vec!["{device:device.description}".parse().unwrap()],
            ..Default::default()
        };

        let device = fixture.state.devices.get(&fixture.device_id).unwrap();
        let result = names.resolve(&fixture.state, device);
        // Should fall back to device name
        assert_eq!(result, Some(String::from("Device name")))
    }

    #[test]
    fn render_endpoint_linked_device() {
        let mut fixture = Fixture::default();

        fixture.state.update(
            &mut fixture.capture_manager,
            MonitorEvent::NodeMediaClass(
                fixture.node_id,
                MediaClass::from("Audio/Sink"),
            ),
        );
        fixture.state.update(
            &mut fixture.capture_manager,
            MonitorEvent::NodeDeviceId(fixture.node_id, fixture.device_id),
        );

        let names = Names {
            endpoint: vec!["{device:device.nick}".parse().unwrap()],
            ..Default::default()
        };

        let node = fixture.state.nodes.get(&fixture.node_id).unwrap();
        let result = names.resolve(&fixture.state, node);
        assert_eq!(result, Some(String::from("Device nick")))
    }

    #[test]
    fn render_endpoint_linked_device_missing_tag() {
        let mut fixture = Fixture::default();

        fixture.state.update(
            &mut fixture.capture_manager,
            MonitorEvent::NodeMediaClass(
                fixture.node_id,
                MediaClass::from("Audio/Sink"),
            ),
        );
        fixture.state.update(
            &mut fixture.capture_manager,
            MonitorEvent::NodeDeviceId(fixture.node_id, fixture.device_id),
        );

        let names = Names {
            endpoint: vec!["{device:device.description}".parse().unwrap()],
            ..Default::default()
        };

        let node = fixture.state.nodes.get(&fixture.node_id).unwrap();
        let result = names.resolve(&fixture.state, node);
        // Should fall back to node name
        assert_eq!(result, Some(String::from("Node name")))
    }

    #[test]
    fn render_endpoint_no_linked_device() {
        let mut fixture = Fixture::default();

        fixture.state.update(
            &mut fixture.capture_manager,
            MonitorEvent::NodeMediaClass(
                fixture.node_id,
                MediaClass::from("Audio/Sink"),
            ),
        );

        let names = Names {
            endpoint: vec!["{device:device.nick}".parse().unwrap()],
            ..Default::default()
        };

        let node = fixture.state.nodes.get(&fixture.node_id).unwrap();
        let result = names.resolve(&fixture.state, node);
        // Should fall back to node name
        assert_eq!(result, Some(String::from("Node name")))
    }

    #[test]
    fn render_stream() {
        let fixture = Fixture::default();

        let names = Names {
            stream: vec!["{node:node.nick}".parse().unwrap()],
            ..Default::default()
        };

        let node = fixture.state.nodes.get(&fixture.node_id).unwrap();
        let result = names.resolve(&fixture.state, node);
        assert_eq!(result, Some(String::from("Node nick")))
    }

    #[test]
    fn render_stream_linked_client() {
        let mut fixture = Fixture::default();

        fixture.state.update(
            &mut fixture.capture_manager,
            MonitorEvent::NodeClientId(fixture.node_id, fixture.client_id),
        );

        let names = Names {
            stream: vec!["{client:application.name}".parse().unwrap()],
            ..Default::default()
        };

        let node = fixture.state.nodes.get(&fixture.node_id).unwrap();
        let result = names.resolve(&fixture.state, node);
        assert_eq!(result, Some(String::from("Client name")))
    }

    #[test]
    fn render_precedence() {
        let fixture = Fixture::default();

        let names = Names {
            stream: vec![
                "{node:node.description}".parse().unwrap(),
                "{node:node.nick}".parse().unwrap(),
            ],
            ..Default::default()
        };

        let node = fixture.state.nodes.get(&fixture.node_id).unwrap();
        let result = names.resolve(&fixture.state, node);
        assert_eq!(result, Some(String::from("Node nick")))
    }

    #[test]
    fn render_override_match() {
        let fixture = Fixture::default();

        let names = Names {
            overrides: vec![NameOverride {
                types: vec![OverrideType::Device, OverrideType::Stream],
                property: Tag::Node(NodeTag::NodeName),
                value: String::from("Node name"),
                templates: vec![
                    "{node:node.description}".parse().unwrap(),
                    "{node:node.nick}".parse().unwrap(),
                ],
            }],
            ..Default::default()
        };

        let node = fixture.state.nodes.get(&fixture.node_id).unwrap();
        let result = names.resolve(&fixture.state, node);
        assert_eq!(result, Some(String::from("Node nick")))
    }

    #[test]
    fn render_override_type_mismatch() {
        let fixture = Fixture::default();

        let names = Names {
            overrides: vec![NameOverride {
                types: vec![OverrideType::Device],
                property: Tag::Node(NodeTag::NodeName),
                value: String::from("Node name"),
                templates: vec!["{node:node.nick}".parse().unwrap()],
            }],
            ..Default::default()
        };

        let node = fixture.state.nodes.get(&fixture.node_id).unwrap();
        let result = names.resolve(&fixture.state, node);
        assert_eq!(result, Some(String::from("Node name")))
    }

    #[test]
    fn render_override_value_mismatch() {
        let fixture = Fixture::default();

        let names = Names {
            overrides: vec![NameOverride {
                types: vec![OverrideType::Device],
                property: Tag::Node(NodeTag::NodeDescription),
                value: String::from("Node name"),
                templates: vec!["{node:node.nick}".parse().unwrap()],
            }],
            ..Default::default()
        };

        let node = fixture.state.nodes.get(&fixture.node_id).unwrap();
        let result = names.resolve(&fixture.state, node);
        assert_eq!(result, Some(String::from("Node name")))
    }

    #[test]
    fn render_override_empty_templates() {
        let fixture = Fixture::default();

        let names = Names {
            overrides: vec![NameOverride {
                types: vec![OverrideType::Device, OverrideType::Stream],
                property: Tag::Node(NodeTag::NodeName),
                value: String::from("Node name"),
                templates: vec![],
            }],
            ..Default::default()
        };

        let node = fixture.state.nodes.get(&fixture.node_id).unwrap();
        let result = names.resolve(&fixture.state, node);
        assert_eq!(result, Some(String::from("Node name")))
    }
}
