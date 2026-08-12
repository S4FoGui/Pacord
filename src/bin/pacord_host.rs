use pacord::capture::CaptureConfig;
use pacord::input::{InputManager, InputPermissions};
use pacord::module2::{run_wayland_host, run_x11_host};
use pacord::overlay::spawn_host_windows;
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;

fn permissions_from_environment() -> InputPermissions {
    let mut permissions = InputPermissions::none();
    for item in env::var("PACORD_ALLOW_INPUT")
        .unwrap_or_default()
        .split(',')
    {
        match item.trim() {
            "keyboard" => permissions.keyboard = true,
            "mouse" | "pointer" => permissions.pointer = true,
            "controller" | "gamepad" => permissions.controller = true,
            "" => {}
            unknown => eprintln!("permissão PACORD desconhecida, ignorada: {unknown}"),
        }
    }
    permissions
}

fn usage() {
    eprintln!(
        "Uso: PACORD_SECRET='<segredo>' cargo run --bin pacord-host -- <wayland|x11> <zerotier-ip:porta>\n\nExemplo:\n  PACORD_SECRET='troque-por-um-segredo-longo' cargo run --bin pacord-host -- x11 10.147.20.5:7777"
    );
}

#[tokio::main]
async fn main() {
    let mut args = env::args().skip(1);
    let Some(backend) = args.next() else {
        usage();
        std::process::exit(2);
    };
    let Some(bind_addr) = args.next() else {
        usage();
        std::process::exit(2);
    };

    let secret = match env::var("PACORD_SECRET") {
        Ok(value) if value.len() >= 16 => value.into_bytes(),
        _ => {
            eprintln!("PACORD_SECRET deve existir e ter pelo menos 16 bytes; nenhum segredo padrão é aceito.");
            std::process::exit(2);
        }
    };
    let bind_addr: SocketAddr = match bind_addr.parse() {
        Ok(value) => value,
        Err(error) => {
            eprintln!("endereço de bind inválido: {error}");
            std::process::exit(2);
        }
    };
    let config = CaptureConfig::default();
    let permissions = permissions_from_environment();
    eprintln!(
        "Permissões PACORD: teclado={}, mouse={}, controle={}",
        permissions.keyboard, permissions.pointer, permissions.controller
    );
    let input_manager = Arc::new(InputManager::new(permissions));
    if env::var_os("DISPLAY").is_some() || env::var_os("WAYLAND_DISPLAY").is_some() {
        spawn_host_windows(input_manager.clone());
    }

    let result = match backend.as_str() {
        "wayland" => run_wayland_host(bind_addr, secret, config, input_manager).await,
        "x11" => run_x11_host(bind_addr, secret, config, input_manager).await,
        _ => {
            usage();
            std::process::exit(2);
        }
    };

    if let Err(error) = result {
        eprintln!("PACORD host encerrou com erro: {error}");
        std::process::exit(1);
    }
}
