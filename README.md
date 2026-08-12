# PACORD

PACORD é um protótipo de colaboração remota consentida para Linux, escrito em Rust, com interface em preto e branco. O produto final foi planejado para usar uma rede ZeroTier já configurada, permitir até oito participantes, exibir cursores identificados por apelido e indicar controles ativos.

## Estado atual

Esta entrega implementa o **Módulo 1: dispositivos virtuais via uinput** e uma interface inicial em `egui`/`eframe`. O módulo cria, por cliente aprovado pelo host, dispositivos virtuais separados para teclado, mouse e controle. Também existe um perfil TOML para sensibilidade do mouse, curva de aceleração, zona morta, inversão do eixo Y e remapeamento de botões.

A tela atual é uma base visual e de gerenciamento: ela mostra a lista de participantes, permissões independentes de mouse/teclado/controle, o limite de oito participantes e a área de sobreposição com apelido e marcador `[PAD]`. A captura real de tela, transporte ZeroTier/TCP, captura local de eventos, autorização criptográfica e sessões isoladas com Cage/Xephyr ficam para os módulos seguintes; portanto, **não trate esta versão como um produto de acesso remoto pronto para produção**.

## Requisitos de segurança e consentimento

O host deve aprovar explicitamente cada participante e cada classe de entrada. O PACORD não deve iniciar uma sessão oculta, contornar autenticação, instalar persistência ou aceitar comandos fora de uma sessão aprovada. A futura camada de rede deve autenticar o host e o cliente, limitar a oito conexões, expirar permissões e exibir um indicador visível enquanto a entrada remota estiver habilitada.

O acesso a `/dev/uinput` é concedido por uma regra local do sistema. O PACORD não deve ser executado permanentemente como root.

## Dependências de compilação

### Debian/Ubuntu

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libudev-dev libasound2-dev \
  libx11-dev libxi-dev libgl1-mesa-dev
```

### Arch Linux

```bash
sudo pacman -S --needed base-devel pkgconf systemd-libs alsa-lib \
  libx11 libxi mesa
```

É necessário ter Rust estável e Cargo disponíveis. Depois, compile com:

```bash
cargo check
cargo test
cargo run
```

## Configuração de uinput sem root permanente

Crie a regra abaixo como `/etc/udev/rules.d/99-pacord-uinput.rules`:

```udev
KERNEL=="uinput", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"
```

A forma recomendada é salvar o arquivo com o nome correto e então adicionar o usuário ao grupo `input`:

```bash
sudo install -m 0644 packaging/99-pacord-uinput.rules /etc/udev/rules.d/99-pacord-uinput.rules
sudo usermod -aG input "$USER"
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=misc
```

Saia e entre novamente na sessão gráfica para que a associação de grupo seja recarregada. Confirme o dispositivo antes de executar o PACORD:

```bash
id -nG | tr ' ' '\n' | grep '^input$'
stat -c '%A %G %n' /dev/uinput
```

Se `/dev/uinput` não existir, carregue o módulo apenas como operação administrativa de configuração:

```bash
sudo modprobe uinput
```

O arquivo de regra só controla permissões do dispositivo local. Ele não abre portas de rede e não configura o ZeroTier.

## Perfil de personalização

Na primeira execução, o perfil `pacord_profile.toml` é criado no diretório de trabalho. Um exemplo equivalente ao perfil padrão é:

```toml
nickname = "PACORD_User"

[mouse]
sensitivity = 1.0
acceleration_curve = 1.0

[controller]
deadzone = 0.15
invert_y = false

[controller.button_map]
A = 304
B = 305
X = 307
Y = 308
```

A aplicação não deve aceitar valores de sensibilidade ou zona morta fora de limites razoáveis na camada de rede. O módulo de configuração é local e deverá ser validado novamente quando a personalização for conectada aos eventos recebidos.

## Arquitetura prevista

| Módulo | Estado | Responsabilidade |
|---|---|---|
| Dispositivos virtuais | Implementado inicialmente | Criar e emitir eventos em teclado, mouse e controle via uinput. |
| Captura local | Planejado | Ler teclado/mouse por evdev e controle por gilrs/SDL2. |
| Host e permissões | Interface inicial | Manter participantes, permissões e limite de oito conexões. |
| Rede | Planejado | Usar IP da rede ZeroTier; a autenticação e o enquadramento de mensagens ainda precisam ser implementados. |
| Captura de tela | Planejado | PipeWire/xdg-desktop-portal em Wayland e XShm como fallback em X11. |
| Sessão isolada | Planejado | Solicitar aprovação antes de iniciar Cage ou Xephyr. |
| Overlay | Demonstração visual | Cursores identificados e marcador de controle na área de visualização. |

## Limitações importantes desta entrega

A versão atual não injeta eventos no desktop até que `/dev/uinput` esteja disponível, não captura a tela, não transmite vídeo e não conecta clientes pela rede. A lista de participantes e os cursores mostrados na interface são dados de demonstração. O próximo incremento deve substituir os dados simulados por um protocolo autenticado, com autorização explícita e testes de integração em X11 e Wayland.

## Licença e desenvolvimento

O repositório original contém outros arquivos não relacionados ao PACORD. O projeto do PACORD está isolado em `pacord/` para que sua evolução possa ser feita com commits pequenos e reversíveis.

## Módulo 2 — captura e transporte

O Módulo 2 adiciona dois backends de captura e um transporte de frames:

| Backend | Sessão | Implementação |
|---|---|---|
| `WaylandPipeWire` | KDE Plasma/Wayland | `ashpd` solicita `CreateSession`, `SelectSources` e `Start` ao portal; o nó autorizado é conectado ao PipeWire por `pipewire-rs`. |
| `X11XShm` | X11 | `x11-dl` verifica MIT-SHM e usa `XShmCreateImage`/`XShmGetImage` para copiar o root window. A ausência da extensão produz erro explícito. |
| Transporte | ZeroTier | `transport.rs` escuta no `SocketAddr` fornecido, executa desafio HMAC-SHA-256, enquadra mensagens com tamanho limitado e distribui JPEGs para no máximo oito clientes. |

O primeiro fluxo validável é composto pelo host executando uma captura autorizada, convertendo cada frame para JPEG e publicando os frames em um broadcast limitado; o cliente envia apelido e prova de posse do segredo compartilhado e recebe apenas frames depois de ser aceito. O segredo não é enviado pela rede. O endereço de bind deve ser o IPv4 da interface ZeroTier, e não `0.0.0.0`.

### Dependências adicionais

No Debian/Ubuntu, além das dependências do Módulo 1, instale:

```bash
sudo apt install -y libpipewire-0.3-dev libspa-0.2-dev libxext-dev \
  libxdamage-dev libclang-dev clang
```

No Arch Linux, instale os equivalentes:

```bash
sudo pacman -S --needed pipewire libpipewire clang libxext libxdamage
```

No Wayland, o fluxo depende de `xdg-desktop-portal`, do backend KDE correspondente e de PipeWire em execução na sessão do usuário. O diálogo de seleção de tela/janela pertence ao portal e precisa ser confirmado localmente pelo host; o PACORD não deve tentar contorná-lo.

### Limitações conscientes

A captura PipeWire aceita inicialmente os formatos brutos RGBA, RGBx, BGRx e RGB; outros formatos negociados são descartados pelo worker até que exista um conversor explícito. A codificação JPEG é uma etapa de validação de latência e largura de banda, não o codec final de produção. A janela interativa completa ainda depende da integração do receptor com a textura da interface e do Módulo 3 de entrada remota; esta etapa transmite vídeo e não autoriza controle por si só.

O protocolo usa autenticação de aplicação além da rede ZeroTier. A rede overlay fornece o caminho IP, mas não concede automaticamente uma sessão PACORD. O host precisa controlar o segredo, aprovar cada cliente e encerrar o processo para interromper todas as transmissões.

### Executar o fluxo host–cliente

Gere um segredo compartilhado fora do processo e entregue-o aos dois participantes por um canal confiável. Não use o valor de exemplo em produção:

```bash
export PACORD_SECRET='substitua-por-um-segredo-com-mais-de-16-bytes'
```

No host X11, usando o IPv4 atribuído pela interface ZeroTier:

```bash
cargo run --release --bin pacord-host -- x11 10.147.20.5:7777
```

No host KDE Wayland, o portal abrirá o diálogo de seleção local:

```bash
cargo run --release --bin pacord-host -- wayland 10.147.20.5:7777
```

Na máquina cliente autorizada, execute a janela visualizadora apontando para o endereço ZeroTier do host:

```bash
cargo run --release --bin pacord-viewer -- 10.147.20.5:7777 Alice
```

O viewer recebe apenas frames após concluir o desafio HMAC-SHA-256. Ele mantém a imagem mais recente em uma janela `egui`; ainda não envia teclado, mouse ou controle, pois essa autorização pertence ao módulo de entrada posterior.

## Referências técnicas

[1] [XDG Desktop Portal — ScreenCast](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html)

[2] [PipeWire — Tutorial: Capturing Video Frames](https://docs.pipewire.org/page_tutorial5.html)

[3] [ashpd — módulo ScreenCast em Rust](https://bilelmoussaoui.github.io/ashpd/ashpd/desktop/screencast/index.html)

[4] [PipeWire Rust bindings](https://pipewire.pages.freedesktop.org/pipewire-rs/pipewire/)

[5] [ZeroTier — Client Configuration](https://docs.zerotier.com/config/)

[6] [X.Org — MIT Shared Memory Extension](https://xorg.freedesktop.org/releases/X11R7.7/doc/xextproto/shm.html)

## Módulo 3 — entrada remota autorizada

O Módulo 3 adiciona um protocolo de entrada separado do stream de vídeo. O host cria um `InputManager` por sessão, registra cada participante com apelido próprio e cria dispositivos virtuais por cliente somente quando chega o primeiro evento autorizado. Teclado, mouse e gamepad são independentes e podem ser habilitados ou revogados separadamente.

A configuração inicial do host usa a variável `PACORD_ALLOW_INPUT`, que aceita `keyboard`, `mouse` e `controller` separados por vírgula. Se a variável não existir, nenhuma entrada remota é permitida:

```bash
export PACORD_ALLOW_INPUT='keyboard,mouse,controller'
export PACORD_SECRET='substitua-por-um-segredo-com-mais-de-16-bytes'
cargo run --bin pacord-host -- x11 10.147.20.5:7777
```

No cliente, a opção **Enviar entrada** só fica efetiva quando o host anuncia uma permissão correspondente. O viewer converte teclas comuns egui para códigos Linux, envia movimento relativo, posição normalizada, botões e roda do mouse, e mapeia eventos básicos do gilrs para eixos e botões do gamepad. A janela também recebe snapshots dos cursores: cada cursor mostra um quadrado branco e uma caixa preta com o nickname; quando há evento de controle, a caixa acrescenta `[PAD]`.

Em uma sessão gráfica do host, o processo abre uma sobreposição transparente, sempre no topo e com mouse passthrough, para mostrar esses mesmos indicadores junto aos cursores reais sem bloquear a área de trabalho. O painel de controle do host permite alterar as três permissões, mostra os participantes ativos e possui o botão `REVOGAR TODAS AS ENTRADAS`. A tecla `F12` executa o mesmo bloqueio de emergência. A revogação destrói as sessões e seus dispositivos virtuais; novas sessões só podem ser registradas depois de `Reativar novas sessões`.

Os eventos são validados antes de chegar ao `uinput`: códigos e valores de tecla/botão são limitados, movimentos excessivos são recusados, posições são normalizadas entre 0 e 1 e eixos de controle são limitados ao intervalo `-32768..=32767`. O transporte mantém o desafio HMAC e o limite de oito clientes. Em Wayland, o backend do portal RemoteDesktop/EIS continua sendo a integração necessária para compositores que não aceitem apenas o caminho uinput; a criação dos dispositivos virtuais é mantida para a compatibilidade do host e para gamepads.

O módulo usa `/dev/uinput` e `/dev/input/uinput`. Garanta que a regra udev do Módulo 1 esteja instalada e que o usuário do serviço tenha acesso ao grupo apropriado. O PACORD não eleva privilégios automaticamente e não concede entrada sem uma permissão explícita do host.

### Referências adicionais do Módulo 3

[7] [Linux Kernel — uinput module](https://www.kernel.org/doc/html/latest/input/uinput.html)

[8] [XDG Desktop Portal — RemoteDesktop](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html)

[9] [libei — EI Protocol documentation](https://libinput.pages.freedesktop.org/libei/)

## Módulo 4 — salas, ZeroTier e interface Plasma

O Módulo 4 substitui a tela simulada por uma aplicação desktop PACORD organizada em quatro áreas: visão geral, criação/gerenciamento de sala, entrada por convite e personalização. O layout foi mantido em preto e branco, com bordas, estados textuais e controles de foco compatíveis com o uso em KDE Plasma Wayland ou X11. O binário continua usando Rust/egui/eframe para preservar a base existente; ele não é um plasmoid e não instala componentes ocultos no Plasma.

Uma sala host possui nome, nickname local, IP ZeroTier, porta, ID da rede, código curto de oito caracteres e um segredo criptográfico aleatório. O convite é um documento TOML que contém o endpoint, o ID da rede, o código e o segredo. Ele deve ser compartilhado por um canal confiável. A sala aceita no máximo oito participantes contando o host; cada participante começa como `PENDING` e o host deve aprová-lo explicitamente. As permissões de teclado, mouse e controle continuam independentes e começam desativadas.

Uma sala cliente é aberta colando o convite na tela **Entrar em sala**. O PACORD valida a versão, o endereço, a porta, o ID de rede, o código e o tamanho do segredo antes de iniciar o viewer. A rede ZeroTier fornece o caminho IP, mas não fornece descoberta de salas; por isso, o convite descentralizado é intencional. O cliente não assume que a conectividade equivale à aprovação do host.

### Diagnóstico e ações administrativas ZeroTier

A aplicação consulta o cliente local com `zerotier-cli -j status` e `zerotier-cli -j listnetworks`, mostra o estado do nó, a versão, o número de redes, dispositivos e os endereços atribuídos. Os botões **Entrar na rede informada** e **Sair da rede informada** executam respectivamente `zerotier-cli join` e `zerotier-cli leave` somente depois de um clique explícito. Dependendo da instalação Linux, esses comandos podem exigir autorização administrativa do usuário; o PACORD exibe o erro e não tenta elevar privilégios automaticamente.

O diagnóstico reconhece os estados `ONLINE`, `OFFLINE` e `TUNNELED`, e exibe falhas como rede não encontrada, autorização pendente ou erro de porta. O PACORD não executa `zerotier-cli dump` automaticamente porque a saída pode conter informações sensíveis do dispositivo.

### Execução do painel

Para iniciar a interface principal durante o desenvolvimento:

```bash
cargo run --bin pacord
```

Para uma instalação local, instale o arquivo desktop do KDE:

```bash
mkdir -p "$HOME/.local/share/applications"
install -m 0644 packaging/org.pacord.PACORD.desktop "$HOME/.local/share/applications/org.pacord.PACORD.desktop"
```

A interface consegue localizar `pacord-host` e `pacord-viewer` como binários irmãos. Para uma instalação em diretórios personalizados, use `PACORD_HOST_BIN` e `PACORD_VIEWER_BIN`. O painel host inicia o backend `wayland` quando `WAYLAND_DISPLAY` está definido; caso contrário, usa `x11`. A captura Wayland ainda solicita a seleção pelo portal local, conforme o Módulo 2.

O Módulo 4 gerencia o ciclo de vida dos processos filhos: iniciar host ou viewer, exibir o estado `INICIANDO`, `EXECUTANDO`, `PARADO` ou `FALHA`, e encerrar a sessão com um botão explícito. Encerrar a sala interrompe o processo da sessão e marca a sala como `CLOSED`; o PACORD não mantém uma sessão oculta em segundo plano.

### Limitações conscientes

A sala é local ao processo e o convite é descentralizado; não há servidor de presença, diretório global ou sincronização de estado entre múltiplos hosts. O cadastro de participante na UI representa a aprovação local; a autenticação efetiva do stream continua sendo o desafio HMAC do transporte. O gerenciamento de autorização no controlador ZeroTier permanece fora do PACORD e deve ser feito pelo administrador da rede quando o status indicar `ACCESS_DENIED`.

O arquivo `module4_research.md` registra as fontes e decisões de arquitetura. As referências principais são o [guia oficial do CLI ZeroTier](https://docs.zerotier.com/cli/), o [guia de desenvolvimento do Plasma](https://community.kde.org/Plasma/DeveloperGuide), a [documentação de desenvolvimento KDE](https://develop.kde.org/docs/) e a [referência de janelas winit](https://docs.rs/winit/latest/winit/window/struct.Window.html).
