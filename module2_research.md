# Pesquisa do Módulo 2 — PACORD

## XDG Desktop Portal ScreenCast
Fonte: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html

A API ScreenCast do XDG Desktop Portal foi projetada para criar sessões de captura consentidas pelo usuário. O fluxo relevante para o PACORD é `CreateSession`, `SelectSources` e `Start`; a documentação afirma que `SelectSources` precisa ocorrer antes de iniciar a sessão e que a sessão pode ser fechada pelo aplicativo ou pelo portal.

Os tipos de fonte documentados incluem monitor, janela e monitor virtual. Os modos de cursor incluem oculto, incorporado no buffer e metadado PipeWire. Para o primeiro incremento, o PACORD deve pedir uma única fonte de monitor/janela, usar o diálogo do portal e só iniciar a transmissão após a resposta de sucesso. O `Start` entrega os dados necessários para conectar ao PipeWire; o identificador do nó não deve ser aceito de cliente remoto nem obtido por varredura do sistema.

## PipeWire
Fonte: https://docs.pipewire.org/page_tutorial5.html

O tutorial oficial de captura mostra um `pw_stream` configurado para vídeo bruto, negociação de `SPA_PARAM_Format`, leitura de tamanho e taxa de quadros e consumo de buffers através de `pw_stream_dequeue_buffer`. O callback de processamento deve retirar o buffer, validar os dados e devolvê-lo ao stream com `pw_stream_queue_buffer`.

A camada PACORD deve separar a captura PipeWire da codificação e do transporte: o callback copia ou referencia o frame apenas durante o processamento, uma fila limitada descarta frames antigos sob pressão e a conexão TCP não pode bloquear o loop PipeWire.

## Decisões de arquitetura

| Área | Decisão do Módulo 2 |
|---|---|
| Wayland/KDE | XDG Desktop Portal ScreenCast + PipeWire, com seleção manual confirmada pelo usuário |
| X11 | Backend XShm em processo separado ou worker dedicado, sem bloquear a UI |
| Transporte | TCP sobre o endereço IPv4 da interface ZeroTier, com enquadramento de mensagens e autenticação de sessão |
| Vídeo inicial | Frames em formato simples e controlado para validar latência; codificação eficiente será isolada para o próximo incremento caso necessário |
| Segurança | Host inicia o compartilhamento visivelmente; cada cliente precisa ser aprovado; parar compartilhamento fecha o stream e derruba clientes |
| Limite | Uma sessão de captura por host e no máximo oito consumidores aprovados |

## Observação

ZeroTier fornece a rede virtual; ele não substitui autenticação de aplicação, autorização por participante ou criptografia específica do protocolo PACORD. A implementação deverá tratar a interface ZeroTier apenas como caminho de transporte e rejeitar conexões fora do endereço configurado.

## ZeroTier
Fonte: https://docs.zerotier.com/config/

A documentação descreve o ZeroTier One como um serviço que cria conectividade por uma porta de rede virtual semelhante a uma VPN. O PACORD deve tratar o endereço ZeroTier como interface/caminho de transporte e não como substituto da autenticação da aplicação. O host deve escutar somente no endereço/interface escolhido, não em todas as interfaces, e o usuário deve informar ou confirmar o endereço do host.

## XShm
Fonte: https://xorg.freedesktop.org/releases/X11R7.7/doc/xextproto/shm.html

A extensão MIT-SHM pode transferir imagens grandes por memória compartilhada local, mas só está disponível em alguns servidores X. A referência recomenda verificar o suporte com `XShmQueryExtension` ou `XShmQueryVersion` e usar fallback convencional quando a extensão não existir. Portanto, o backend X11 do PACORD deve detectar a extensão, capturar em um worker e retornar erro orientado ao usuário quando XShm não estiver disponível, sem presumir que toda sessão X11 possui a extensão.

## Bindings Rust

Fonte ashpd: https://bilelmoussaoui.github.io/ashpd/ashpd/desktop/screencast/index.html

A documentação do `ashpd` 0.13.0 fornece um fluxo Rust com `Screencast::new`, `create_session`, `select_sources`, `start` e leitura dos streams retornados. Ela também expõe `open_pipe_wire_remote`; o exemplo confirma que a seleção de fonte ocorre pelo portal e que o resultado inclui `pipe_wire_node_id`, tamanho e posição.

Fonte pipewire-rs: https://pipewire.pages.freedesktop.org/pipewire-rs/pipewire/

As bindings Rust organizam o cliente em `MainLoop`, `Context`, `Core`, opcionalmente `Registry` e `Stream`. O loop despacha callbacks e chamadas; callbacks precisam ser compatíveis com o ciclo de vida exigido pela biblioteca. O worker PipeWire do PACORD deverá ser dedicado e comunicar frames por canal limitado ao restante da aplicação.
