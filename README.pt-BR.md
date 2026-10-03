# Win-CodexBar

[English](./README.md) | [简体中文](./README.zh-CN.md) | [繁體中文（臺灣）](./README.zh-TW.md) | [日本語](./README.ja-JP.md) | [한국어](./README.ko-KR.md) | [Español mexicano](./README.es-MX.md) | [Português (Brasil)](./README.pt-BR.md) | [Türkçe](./README.tr-TR.md)

O Win-CodexBar é um aplicativo para a bandeja do sistema do Windows que mantém visível o uso das suas ferramentas de programação com IA, sem precisar abrir vários painéis. Ele traz a proposta do [CodexBar](https://github.com/steipete/CodexBar) para uma aplicação desktop em Tauri + React, apoiada por uma lógica compartilhada de provedores escrita em Rust.

<table>
  <tr>
    <td width="36%" align="center">
      <img src="docs/images/tray-panel.png" alt="Painel da bandeja do Win-CodexBar mostrando cartões de uso dos provedores"/>
    </td>
    <td width="64%" align="center">
      <img src="docs/images/settings-providers.png" alt="Página de configurações de provedores do Win-CodexBar"/>
    </td>
  </tr>
</table>

## Destaques

- **51 provedores**, incluindo Codex, Claude, Copilot, OpenRouter, Cursor, Gemini, DeepSeek, MiniMax, Kiro, Antigravity, Groq e muitos outros.
- **Fluxo centrado na bandeja**, com uma grade compacta de provedores, cartões de uso, ação de atualização, atalho para as configurações e controle para sair.
- **Configurações por provedor** para selecionar fontes de dados, credenciais, importação de cookies, contas de token, chaves de API, regiões e preferências de exibição na bandeja.
- **Proteção de credenciais no Windows** para chaves de API, cookies manuais e contas de token gerenciados pelo aplicativo, usando DPAPI no escopo do usuário quando disponível.
- **Importação de cookies do navegador** para Chrome, Edge, Brave e Firefox, ativada individualmente para cada provedor.
- **CLI local instalada** para automatizar consultas de uso, custos, configurações, diagnósticos e integrações via loopback.
- **Versões instalável e portátil**, com inicialização do runtime do WebView2 e do runtime do VC++, além de arquivos de verificação SHA-256.

## Instalação

Instale com o Gerenciador de Pacotes do Windows:

```powershell
winget install Finesssee.Win-CodexBar
```

Ou baixe a versão mais recente do instalador ou do executável portátil em [GitHub Releases](https://github.com/nesszer/Win-CodexBar/releases).

- Instalador: `CodexBar-<version>-Setup.exe`
- Portátil: `CodexBar-<version>-portable.exe`
- Checksums: cada versão inclui arquivos `.sha256`

A distribuição pelo Winget foi aprovada no [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs/tree/master/manifests/f/Finesssee/Win-CodexBar). Novas versões podem demorar um pouco para aparecer, pois cada atualização no Winget é vinculada a uma URL de versão e a um hash específico do instalador.

## Assinatura de código

> **Assinatura de código:** o SignPath.io está integrado ao fluxo de lançamento do GitHub Actions, mas a assinatura em produção permanece bloqueada até a emissão do certificado Release 2026 e a validação da política `release-signing`. Consulte [docs/CODE_SIGNING.md](docs/CODE_SIGNING.md) para conhecer a política de assinatura.
> A versão v0.60.3 é imutável e não está assinada; verifique os arquivos SHA-256. A primeira versão assinada será a próxima versão normal após a integração com o SignPath.

## Primeira execução

1. Inicie o **CodexBar** pelo menu Iniciar ou pelo executável portátil.
2. Clique no ícone da bandeja para abrir o painel de uso.
3. Abra **Configurações -> Provedores**.
4. Ative os provedores que você utiliza.
5. Adicione o tipo de credencial correspondente: login OAuth/dispositivo, chave de API, cookies do navegador, login na CLI local ou conta de token.

Para o Claude, cookies do navegador/sessionKey são recomendados porque correspondem ao uso exibido na página de configurações do Claude. OAuth e CLI continuam disponíveis como alternativas. Para provedores baseados em CLI, como Codex e Gemini, primeiro faça login pela CLI do provedor.

## Histórico de versões

Consulte todas as alterações em [CHANGELOG.md](CHANGELOG.md).

## Provedores compatíveis

<details>
<summary>Matriz de provedores</summary>

| Provedor | Autenticação | Dados monitorados |
|---|---|---|
| Codex | OAuth / CLI | Sessão, semanal, créditos |
| Claude | Cookies / alternativa OAuth / alternativa CLI | Sessão (5h), semanal |
| Cursor | Cookies | Plano, uso, cobrança |
| Factory | Cookies | Uso |
| Gemini | OAuth do gcloud | Cota |
| Copilot | Fluxo de dispositivo do GitHub / CLI gh / token legado | Uso do plano, Chat |
| Antigravity | LSP local | Uso, cotas por modelo |
| z.ai | Token de API | Cota |
| MiniMax | API / Cookies | Uso, resumo de cobrança |
| Kiro | Cookies / CLI | Créditos mensais, excedentes |
| Vertex AI | OAuth do gcloud | Custo |
| v0 | Chave de API | Cota de cobrança, limites de requisição da API, saldo sob demanda |
| Augment | Cookies | Créditos |
| OpenCode | Configuração local | Uso |
| Kimi | Cookies | Limite de 5h, semanal |
| Kimi K2 | Chave de API | Créditos |
| Amp | Cookies | Uso |
| Warp | Configuração local | Uso |
| Ollama | Cookies / Chave de API | Uso, modelos na nuvem, janelas de ritmo |
| Azure OpenAI | Chave de API | Implantação |
| T3 Chat | Cookies / cURL | Base, excedentes |
| TypeSafe | Cookies do navegador / cabeçalho Cookie manual | Gastos do ciclo de cobrança, saldo, créditos a expirar |
| OpenRouter | Chave de API | Créditos |
| JetBrains AI | Configuração local | Uso |
| Alibaba | Cookies | Uso |
| Alibaba Token Plan | Cookies | Créditos do plano de tokens, data de renovação |
| NanoGPT | Chave de API | Créditos |
| Infini | Chave de API | Sessão, semanal, cota |
| Perplexity | Cookies | Créditos, plano |
| Abacus AI | Cookies | Créditos |
| Mistral | Cookies | Cobrança, uso |
| OpenCode Go | Cookies | Uso, saldo Zen |
| Kilo | Chave de API / CLI | Uso |
| Codebuff | Chave de API / Configuração local | Créditos, semanal |
| DeepSeek | Chave de API | Saldo, resumos de uso, custo |
| Windsurf | Cache local | Diário, semanal |
| Manus | Cookies | Créditos, créditos de atualização |
| Xiaomi MiMo | Cookies | Saldo, plano de tokens |
| Doubao | Chave de API | Limites de requisição |
| Command Code | Cookies | Créditos mensais, créditos comprados |
| StepFun | Token Oasis | 5h, semanal, atualização de token |
| Venice | Chave de API / sessão web | Saldo em USD / DIEM, créditos incluídos (a sessão web expira em cerca de 60 s) |
| OpenAI | API Admin / Chave de API | Uso, requisições, custo por projeto, saldo de créditos |
| Grok | Cookies / auth.json | Cobrança |
| Helmcode (também NaN Builders) | Cookies do navegador / cabeçalho Cookie manual | Cotas de tokens por modelo, janelas de renovação, saldo pré-pago do Helmcode |
| Replicate | Cookies / contas de token | Gastos mensais, saldo de créditos |
| Aixy | Chave de API / contas de token | Saldos de orçamento aplicáveis, uso da chave em 7 dias |
| ElevenLabs | Chave de API | Créditos da assinatura, slots de voz |
| Deepgram | Chave de API | Uso do projeto |
| Groq | Chave de API | Métricas corporativas |
| LLM Proxy | Chave de API | Estatísticas de cota |

</details>

## Idiomas disponíveis

A interface e os relatórios para colaboradores estão disponíveis atualmente em:

- English
- 简体中文
- 繁體中文（臺灣）
- 日本語
- 한국어
- Español mexicano
- Português (Brasil)
- Türkçe

## Compilação a partir do código-fonte

```powershell
# Pré-requisitos: Node.js + pnpm. Rust e MinGW são instalados pelo script quando necessário.
git clone https://github.com/nesszer/Win-CodexBar.git
cd Win-CodexBar
.\scripts\dev.ps1
```

Opções úteis para desenvolvimento:

```powershell
.\scripts\dev.ps1 -Release      # compilação otimizada
.\scripts\dev.ps1 -SkipBuild    # reinicia a última compilação
```

Exemplos da CLI:

```bash
codexbar-cli --help
codexbar-cli diagnose --pretty
codexbar-cli usage -p claude
codexbar-cli usage -p all
codexbar-cli cost -p codex
```

As compilações do instalador incluem `codexbar.exe` como aplicativo da bandeja e `codexbar-cli.exe` como CLI de console. Os atalhos do menu Iniciar executam o aplicativo desktop; os comandos do terminal usam `codexbar-cli.exe`. O `codexbar-desktop.exe` ainda é instalado como um alias de compatibilidade para atalhos antigos e entradas de inicialização automática.

## Compilações de lançamento

Para criar versões locais no Windows, use o gerador de versões com cache:

```powershell
.\scripts\windows-release-build.ps1 -Ref v0.33.2 -SmokeInstall
```

O script compila o binário de lançamento real do Tauri e a CLI de console, verifica as dependências assinadas do instalador, empacota com o Inno Setup, gera os artefatos instalável e portátil, cria os arquivos SHA-256 correspondentes e pode executar um teste rápido de instalação e desinstalação silenciosas.

Mais informações sobre a automação de versões estão em [docs/release/ci-cd.md](docs/release/ci-cd.md).

## Privacidade

- **Processamento local por padrão**: os dados dos provedores são lidos de caminhos locais conhecidos ou das APIs que você configurar.
- **Cookies opcionais**: a extração de cookies do navegador só é executada para os provedores ativados por você.
- **Segredos protegidos**: chaves de API, cookies manuais e contas de token utilizam a camada de arquivo seguro; no Windows, ela usa DPAPI no escopo do usuário quando disponível.
- **Diagnósticos seguros**: os diagnósticos mostram apenas metadados de provedor, fonte e status, nunca cookies, chaves de API, bearer tokens ou valores OAuth brutos.
- **Atualizações verificadas**: downloads do instalador exigem um resumo SHA-256 do GitHub e são verificados novamente imediatamente antes da aplicação.

## Documentação

| Tópico | Link |
|---|---|
| Compilação a partir do código-fonte | [docs/BUILDING.md](docs/BUILDING.md) |
| Configuração do WSL e dicas de autenticação | [docs/WSL.md](docs/WSL.md) |
| Detalhes dos cookies do navegador | [docs/COOKIES.md](docs/COOKIES.md) |

## Integrações locais

- [AI Usage Limits](https://github.com/lenadweb/stream-deck-ai-limits) — integração com o Elgato Stream Deck que utiliza o painel/API local de `codexbar serve` para exibir métricas de provedor, conta, cota ou payload.
- [AI Monitor](https://github.com/tobymarks/esp32-ai-monitor) — monitor de mesa com ESP32 (Cheap Yellow Display) e aplicativo complementar para Windows (beta), que chama a CLI instalada (`codexbar-cli usage -p <provider> --json`) e transmite os limites de Claude, Codex, Copilot, Cursor, Gemini ou Antigravity para o monitor por uma conexão serial USB.

## Créditos

- Aplicativo original para macOS: [steipete/CodexBar](https://github.com/steipete/CodexBar), de Peter Steinberger
- Inspirado no [ccusage](https://github.com/ryoppippi/ccusage) para o acompanhamento de custos

## Licença

MIT, a mesma licença do CodexBar original.
