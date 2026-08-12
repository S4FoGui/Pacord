use evdev::{
    uinput::VirtualDeviceBuilder, AbsInfo, AbsoluteAxisType, AttributeSet, EventType, InputEvent,
    Key, RelativeAxisType, UinputAbsSetup,
};
use std::error::Error;

pub struct ClientVirtualDevices {
    pub client_id: usize,
    pub nickname: String,
    keyboard: evdev::uinput::VirtualDevice,
    mouse: evdev::uinput::VirtualDevice,
    gamepad: evdev::uinput::VirtualDevice,
}

impl ClientVirtualDevices {
    pub fn new(client_id: usize, nickname: &str) -> Result<Self, Box<dyn Error>> {
        let dev_name_kb = format!("PACORD Client {} ({}) Keyboard", client_id, nickname);
        let mut keys = AttributeSet::<Key>::new();
        // Enable common keys
        for code in 1..255 {
            keys.insert(Key::new(code));
        }

        let keyboard = VirtualDeviceBuilder::new()?
            .name(&dev_name_kb)
            .with_keys(&keys)?
            .build()?;

        let dev_name_mouse = format!("PACORD Client {} ({}) Mouse", client_id, nickname);
        let mut rels = AttributeSet::<RelativeAxisType>::new();
        rels.insert(RelativeAxisType::REL_X);
        rels.insert(RelativeAxisType::REL_Y);
        rels.insert(RelativeAxisType::REL_WHEEL);

        let mut mouse_keys = AttributeSet::<Key>::new();
        mouse_keys.insert(Key::BTN_LEFT);
        mouse_keys.insert(Key::BTN_RIGHT);
        mouse_keys.insert(Key::BTN_MIDDLE);

        let mouse = VirtualDeviceBuilder::new()?
            .name(&dev_name_mouse)
            .with_keys(&mouse_keys)?
            .with_relative_axes(&rels)?
            .build()?;

        let dev_name_gp = format!("PACORD Client {} ({}) Gamepad", client_id, nickname);
        let mut gp_keys = AttributeSet::<Key>::new();
        for code in 0x130..0x140 {
            gp_keys.insert(Key::new(code));
        }

        let gamepad_builder = VirtualDeviceBuilder::new()?
            .name(&dev_name_gp)
            .with_keys(&gp_keys)?;

        let gamepad_builder = gamepad_builder
            .with_absolute_axis(&UinputAbsSetup::new(
                AbsoluteAxisType::ABS_X,
                AbsInfo::new(0, -32768, 32767, 0, 0, 0),
            ))?
            .with_absolute_axis(&UinputAbsSetup::new(
                AbsoluteAxisType::ABS_Y,
                AbsInfo::new(0, -32768, 32767, 0, 0, 0),
            ))?
            .with_absolute_axis(&UinputAbsSetup::new(
                AbsoluteAxisType::ABS_RX,
                AbsInfo::new(0, -32768, 32767, 0, 0, 0),
            ))?
            .with_absolute_axis(&UinputAbsSetup::new(
                AbsoluteAxisType::ABS_RY,
                AbsInfo::new(0, -32768, 32767, 0, 0, 0),
            ))?;

        let gamepad = gamepad_builder.build()?;

        Ok(Self {
            client_id,
            nickname: nickname.to_string(),
            keyboard,
            mouse,
            gamepad,
        })
    }

    pub fn send_key_event(&mut self, key_code: u16, value: i32) -> Result<(), Box<dyn Error>> {
        let ev = InputEvent::new(EventType::KEY, key_code, value);
        self.keyboard.emit(&[ev])?;
        Ok(())
    }

    pub fn send_mouse_motion(&mut self, dx: i32, dy: i32) -> Result<(), Box<dyn Error>> {
        let mut events = vec![];
        if dx != 0 {
            events.push(InputEvent::new(
                EventType::RELATIVE,
                RelativeAxisType::REL_X.0,
                dx,
            ));
        }
        if dy != 0 {
            events.push(InputEvent::new(
                EventType::RELATIVE,
                RelativeAxisType::REL_Y.0,
                dy,
            ));
        }
        if !events.is_empty() {
            self.mouse.emit(&events)?;
        }
        Ok(())
    }

    pub fn send_mouse_button(
        &mut self,
        button_code: u16,
        value: i32,
    ) -> Result<(), Box<dyn Error>> {
        let ev = InputEvent::new(EventType::KEY, button_code, value);
        self.mouse.emit(&[ev])?;
        Ok(())
    }

    pub fn send_mouse_wheel(&mut self, value: i32) -> Result<(), Box<dyn Error>> {
        let ev = InputEvent::new(EventType::RELATIVE, RelativeAxisType::REL_WHEEL.0, value);
        self.mouse.emit(&[ev])?;
        Ok(())
    }

    pub fn send_gamepad_axis(
        &mut self,
        axis: AbsoluteAxisType,
        value: i32,
    ) -> Result<(), Box<dyn Error>> {
        let ev = InputEvent::new(EventType::ABSOLUTE, axis.0, value);
        self.gamepad.emit(&[ev])?;
        Ok(())
    }

    pub fn send_gamepad_button(
        &mut self,
        button_code: u16,
        value: i32,
    ) -> Result<(), Box<dyn Error>> {
        let ev = InputEvent::new(EventType::KEY, button_code, value);
        self.gamepad.emit(&[ev])?;
        Ok(())
    }
}
