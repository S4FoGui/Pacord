# Pesquisa do Módulo 4 — ZeroTier e KDE Plasma

## ZeroTier CLI

Fonte oficial: https://docs.zerotier.com/cli/

A documentação oficial informa que o CLI local gerencia o nó e a associação do cliente às redes. O PACORD poderá consultar `zerotier-cli status`, `zerotier-cli -j listnetworks` e `zerotier-cli -j listpeers`; o parâmetro `-j` é apropriado para parsing estruturado. Os comandos administrativos `join <network-id>` e `leave <network-id>` devem ser tratados como ações explícitas, porque em sistemas Unix podem exigir privilégios elevados e o cliente precisa estar autorizado no controlador da rede.

O status do nó pode ser ONLINE, OFFLINE ou TUNNELED. Para uma sala PACORD, a rede ZeroTier fornece conectividade IP, mas não cria descoberta de salas. O convite deverá conter o endereço ZeroTier do host, a porta PACORD, o código da sala e o segredo compartilhado. A UI deverá mostrar os erros `REQUESTING_CONFIGURATION`, `NOT_FOUND`, `ACCESS_DENIED` e `PORT_ERROR` sem tentar contorná-los.

O PACORD não deve executar `dump` automaticamente, pois a documentação alerta que o arquivo pode conter endereços físicos e identidades públicas sensíveis.

## Decisões preliminares

A integração será feita por um wrapper Rust que executa o binário local somente após ação explícita do usuário. A leitura de status e redes será não destrutiva; `join` e `leave` terão confirmação na UI e exibirão a saída do comando. Nenhuma ação administrativa será executada silenciosamente no startup.

A interface nativa continuará em egui/eframe, com nome de aplicativo e janelas adequados a Linux/Wayland/X11. A aparência será estritamente preto e branco, com estados transmitidos por texto, bordas e ícones monocromáticos, evitando depender de um toolkit Qt adicional para a primeira entrega.

## KDE Plasma e interface nativa

Fontes consultadas:

- https://community.kde.org/Plasma/DeveloperGuide
- https://develop.kde.org/docs/
- https://docs.rs/winit/latest/winit/window/struct.Window.html

O guia do KDE descreve aplicativos Plasma como aplicações em sua própria janela e recomenda atenção a análise de requisitos, design de interação, consistência visual, acessibilidade e integração de desktop. A documentação atual do KDE também aponta caminhos para Kirigami, KXmlGui, Rust com Kirigami, configuração via KConfig e comunicação D-Bus. O PACORD continuará em egui/eframe nesta etapa para preservar a base Rust existente, mas adotará identidade de aplicativo Linux consistente, navegação por estados, foco de teclado, textos claros e layout adaptável.

A referência do winit confirma que a camada de janela usada pelo eframe oferece atributos para título, tamanho, transparência e integração Wayland/X11. O Módulo 4 deve usar um título PACORD estável, uma janela principal não transparente para a sala e uma janela de diagnóstico separada apenas quando necessário; a sobreposição do Módulo 3 permanece uma janela distinta e passiva.

Decisão de escopo: a primeira versão não será um plasmoid nem adicionará QML/Qt ao binário principal. Ela será uma aplicação desktop PACORD bem integrada ao Plasma, deixando a migração opcional para Kirigami como refinamento posterior, caso o usuário queira integração com componentes KDE específicos.
