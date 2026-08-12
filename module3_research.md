# Pesquisa do Módulo 3

## Kernel uinput
Fonte: https://www.kernel.org/doc/html/latest/input/uinput.html

A documentação do kernel define uinput como um módulo que permite emular dispositivos de entrada a partir do espaço do usuário. O processo abre `/dev/uinput` ou `/dev/input/uinput`, configura capacidades do dispositivo virtual e, após `UI_DEV_CREATE`, escreve eventos que são entregues aos consumidores do kernel e do espaço do usuário. A própria documentação recomenda considerar libevdev para software novo, por reduzir erros em comparação com chamadas uinput diretas.

Os exemplos oficiais cobrem teclado com `EV_KEY`, mouse com botões `EV_KEY` e movimento relativo `EV_REL`/`REL_X`/`REL_Y`, além do ciclo de criação, envio de `SYN_REPORT` e destruição do dispositivo.

## XDG RemoteDesktop
Fonte: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html

O portal RemoteDesktop é a API de desktop remoto do XDG para criar uma sessão autorizada pelo usuário. A propriedade `AvailableDeviceTypes` define atualmente `KEYBOARD`, `POINTER` e `TOUCHSCREEN`. O fluxo da API é `CreateSession`, `SelectDevices` e `Start`; a autorização é devolvida por meio das respostas do portal. A especificação não lista gamepad como um tipo de dispositivo do RemoteDesktop v2.

Decisão provisória: o PACORD pode manter `uinput` como backend de dispositivos virtuais no host, desde que a permissão seja explícita e revogável. Para Wayland, deve-se avaliar o portal RemoteDesktop/EIS para teclado e ponteiro quando a política do compositor exigir uma sessão portal; `uinput` continua sendo o caminho para criar dispositivos virtuais isolados. Gamepad exigirá um dispositivo virtual separado via uinput, pois não aparece como tipo do portal RemoteDesktop documentado.

## libei/EIS
Fonte: https://libinput.pages.freedesktop.org/libei/

libei é uma biblioteca de entrada emulada voltada principalmente à pilha Wayland. O modelo separa o cliente EI, que se comporta como uma fonte de entrada, do servidor EIS, normalmente o compositor Wayland, conectados por socket Unix. Os eventos emulados podem ser distinguidos pelo compositor para aplicar controle fino sobre quando e quais eventos são permitidos; para os clientes Wayland, quando aceitos, eles entram na pilha de entrada como eventos equivalentes aos dispositivos físicos.

Decisão de implementação: o PACORD mantém o caminho uinput por participante para a primeira entrega e registra a necessidade de adaptar o host Wayland para `RemoteDesktop.ConnectToEIS`/libei quando o backend KDE oferecer essa integração. O gamepad não aparece entre os tipos documentados do portal RemoteDesktop v2, portanto permanece como dispositivo virtual uinput separado.
