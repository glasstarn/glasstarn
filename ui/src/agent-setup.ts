// Agent setup: connecting an AI agent to the open project.

let agentSetup = null;

function agentScopeText() {
  const named = document.querySelector("#agent-preset").value === "business_names";
  return `${named ? "It sees step and column names; case and people IDs stay hidden." : "Names and IDs are replaced with labels."} It sees counts and durations, never your raw rows, file names or exact dates.`;
}

function renderAgentSetup(data) {
  agentSetup = data;
  document.querySelector("#agent-enable").disabled = data.enabled;
  document.querySelector("#agent-disable").disabled = !data.enabled;
  document.querySelector("#agent-enable").hidden = data.enabled;
  document.querySelector("#agent-disable").hidden = !data.enabled;
  document.querySelector("#agent-preset").disabled = data.enabled;
  document.querySelector("#agent-setup").hidden = !data.enabled;
  const preset = document.querySelector("#agent-preset");
  if (data.enabled) preset.value = data.policy?.activity_names === "show" ? "business_names" : "private";
  document.querySelector("#agent-scope").textContent = agentScopeText();
  document.querySelector("#agent-policy").textContent = data.policy ? JSON.stringify(data.policy, null, 2) : "Access is off.";
  document.querySelector("#agent-access-status").textContent = !data.enabled ? "" : data.analysis_ready ? `Access is on for ${data.project_name}.` : `Access is on for ${data.project_name}. Import an event log so your agent has results to read.`;
  document.querySelector("#agent-access-status").dataset.state = "ready";
  document.querySelector("#agent-prompt").value = data.prompt;
  const contact = data.contact;
  const seen = contact?.last_seen ? new Date(contact.last_seen * 1000).toLocaleString() : "";
  document.querySelector("#agent-contact").textContent = !data.enabled ? "" : contact?.initialized
    ? `Connected to ${contact.client || "your agent"}. Last contact ${seen}.`
    : "Not connected yet.";
  const button = document.querySelector("#app-agent");
  if (button) button.textContent = data.enabled && contact?.initialized ? "Agent connected" : "Connect an agent";
  renderAgentClient();
}

// Grows a copy box to show its whole text, so nothing is cut off mid-line.
function fitAgentBox(field) {
  if (!field?.style) return;
  field.style.height = "auto";
  field.style.height = `${Math.min(field.scrollHeight + 2, 240)}px`;
}

function renderAgentClient() {
  if (!agentSetup) return;
  const client = document.querySelector("#agent-client").value;
  const instructions = document.querySelector("#agent-instructions");
  const config = document.querySelector("#agent-config");
  const extra = document.querySelector("#agent-extra");
  extra.hidden = client !== "codex" && client !== "claude";
  if (client === "codex") {
    instructions.textContent = `Run this once in ${agentSetup.shell}.`;
    config.value = agentSetup.codex_command;
    document.querySelector("#agent-extra-instructions").textContent = "Add this to your Codex config.toml, keeping what is already there.";
    document.querySelector("#agent-extra-config").value = agentSetup.codex_config;
  } else if (client === "claude") {
    instructions.textContent = `Run this once in ${agentSetup.shell}, in the folder where you use Claude Code.`;
    config.value = agentSetup.claude_command;
    document.querySelector("#agent-extra-instructions").textContent = "Add this to the .mcp.json file in that folder, keeping what is already there.";
    document.querySelector("#agent-extra-config").value = JSON.stringify(agentSetup.claude_config, null, 2);
  } else if (client === "vscode") {
    instructions.textContent = "In VS Code, add this to .vscode/mcp.json, then turn on its tools in Copilot Chat.";
    config.value = JSON.stringify(agentSetup.vscode_config, null, 2);
  } else {
    instructions.textContent = "Add this as a local MCP server in your agent.";
    config.value = JSON.stringify(agentSetup.generic_config, null, 2);
  }
  fitAgentBox(config);
  fitAgentBox(document.querySelector("#agent-prompt"));
}

async function refreshAgentSetup() {
  try { renderAgentSetup(await workspaceApi("/api/agent")); }
  catch (error) { agentSetupError(error); }
}
function agentSetupError(error) {
  const status = document.querySelector("#agent-access-status");
  status.textContent = error.message || "The agent connection could not be updated.";
  status.dataset.state = "error";
}
async function changeAgentAccess(enable) {
  const button = document.querySelector(enable ? "#agent-enable" : "#agent-disable");
  button.disabled = true;
  try {
    const body = enable ? { preset: document.querySelector("#agent-preset").value } : {};
    renderAgentSetup(await workspaceApi(`/api/agent/${enable ? "enable" : "disable"}`, body));
  } catch (error) { button.disabled = false; agentSetupError(error); }
}
async function copyAgentText(selector) {
  const field = document.querySelector(selector);
  try { await navigator.clipboard.writeText(field.value); }
  catch (_error) { field.focus(); field.select(); agentSetupError(new Error("Copy is unavailable. The text is selected; copy it manually.")); }
}
async function initializeAgentSetup() {
  document.querySelector("#agent-panel").hidden = false;
  document.querySelector("#agent-enable").addEventListener("click", () => void changeAgentAccess(true));
  document.querySelector("#agent-disable").addEventListener("click", () => void changeAgentAccess(false));
  document.querySelector("#agent-check").addEventListener("click", () => void refreshAgentSetup());
  document.querySelector("#agent-client").addEventListener("change", renderAgentClient);
  // Boxes can only be measured once the panel is open.
  document.querySelector("#agent-panel").addEventListener("toggle", renderAgentClient);
  document.querySelector("#agent-preset").addEventListener("change", () => { document.querySelector("#agent-scope").textContent = agentScopeText(); });
  document.querySelector("#agent-copy-config").addEventListener("click", () => void copyAgentText("#agent-config"));
  document.querySelector("#agent-copy-prompt").addEventListener("click", () => void copyAgentText("#agent-prompt"));
  await refreshAgentSetup();
}
