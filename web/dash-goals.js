/*
 * 目标与待办面板。
 *
 * 上面是跨会话的目标总览(/api/goals):每行一个会话的当前目标,phase 徽章
 * + 轮次计数 + 操作按钮(pause/resume/complete)。操作带 CAS 凭证
 * (goal_id + revision),撞版本(409)就整表重拉。
 * 下面是 daemon 级轻量待办(/api/todos):增删、勾完成,与目标同面板展示。
 */
(() => {
  const D = window.MiyuDash;
  if (!D) return;

  const state = { goals: [], todos: [], loaded: false };
  const ui = {};

  const PHASES = {
    active: { label: t("进行中"), cls: "is-ok" },
    paused: { label: t("已暂停"), cls: "is-warn" },
    blocked: { label: t("受阻"), cls: "is-danger" },
    complete: { label: t("已完成"), cls: "" }
  };
  const PRIORITIES = ["high", "medium", "low"];
  const PRIORITY_LABELS = { high: t("高"), medium: t("中"), low: t("低") };

  const preview = (text, n = 120) => {
    const s = String(text || "");
    return s.length > n ? s.slice(0, n) + "…" : s;
  };

  /* ── 目标列表 ─────────────────────────────────────────── */

  async function loadGoals() {
    const data = await D.api("/api/goals");
    state.goals = data.goals || [];
  }

  async function goalAction(row, action) {
    try {
      await D.api(`/api/goals/${encodeURIComponent(row.session_id)}/${action}`, {
        method: "POST",
        body: { goal_id: row.goal_id, revision: row.revision }
      });
      D.toast(t("已{action}", { action: action === "pause" ? t("暂停") : action === "resume" ? t("恢复") : t("完成") }));
    } catch (error) {
      D.toast(t("失败:{message}", { message: error.message }), "error");
    }
    await refresh();
  }

  function goalRow(row) {
    const phase = PHASES[row.phase] || { label: row.phase, cls: "" };
    const buttons = D.el("div.dash-goal-actions");
    const blocked = row.phase === "blocked";
    const rounds = row.max_rounds > 0
      ? t("第 {n} / {max} 轮", { n: row.rounds_started, max: row.max_rounds })
      : t("第 {n} 轮", { n: row.rounds_started });
    if (row.phase === "active") {
      buttons.append(
        D.el("button.dash-button", { type: "button", text: t("暂停"), onclick: () => goalAction(row, "pause") }),
        D.el("button.dash-button.is-primary", { type: "button", text: t("完成"), onclick: () => goalAction(row, "complete") }));
    } else if (blocked || row.phase === "paused") {
      buttons.append(
        D.el("button.dash-button.is-primary", { type: "button", text: t("继续"), onclick: () => goalAction(row, "resume") }),
        D.el("button.dash-button", { type: "button", text: t("完成"), onclick: () => goalAction(row, "complete") }));
    } else {
      buttons.append(D.el("span.dash-card-hint", { text: t("收工") }));
    }
    return D.el("div.dash-goal-row", null,
      D.el("div.dash-goal-main", null,
        D.el("div.dash-goal-title", null,
          D.el("span.dash-chip." + (phase.cls || "is-muted"), { text: phase.label }),
          D.el("strong", { text: row.session_name || row.session_id }),
          D.el("small", { text: rounds }),
          D.el("small", { text: D.formatTime(row.updated_at) })),
        D.el("p.dash-goal-objective", { text: preview(row.objective), title: row.objective }),
        row.blocked_message ? D.el("p.dash-goal-blocked", { text: t("卡在:{message}", { message: row.blocked_message }) }) : null),
      buttons);
  }

  function renderGoals() {
    ui.goalList.textContent = "";
    if (!state.goals.length) {
      ui.goalList.append(D.el("p.dash-empty", { text: t("还没有目标。在会话里说一句「/goal 目标」或让模型调 goal 工具开始一件长任务。") }));
      return;
    }
    for (const row of state.goals) ui.goalList.append(goalRow(row));
  }

  /* ── 待办清单 ─────────────────────────────────────────── */

  async function loadTodos() {
    const data = await D.api("/api/todos");
    state.todos = data.todos || [];
  }

  async function addTodo() {
    const content = ui.todoInput.value.trim();
    if (!content) return;
    const priority = ui.todoPriority.value;
    ui.todoInput.value = "";
    try {
      await D.api("/api/todos", { method: "POST", body: { content, priority } });
      await loadTodos();
      renderTodos();
    } catch (error) {
      D.toast(t("失败:{message}", { message: error.message }), "error");
    }
  }

  async function toggleTodo(row) {
    try {
      await D.api(`/api/todos/${row.id}`, { method: "PATCH", body: { status: row.status === "done" ? "open" : "done" } });
      await loadTodos();
      renderTodos();
    } catch (error) {
      D.toast(t("失败:{message}", { message: error.message }), "error");
    }
  }

  async function deleteTodo(row) {
    try {
      await D.api(`/api/todos/${row.id}`, { method: "DELETE" });
      state.todos = state.todos.filter((item) => item.id !== row.id);
      renderTodos();
    } catch (error) {
      D.toast(t("失败:{message}", { message: error.message }), "error");
    }
  }

  function todoRow(row) {
    const done = row.status === "done";
    const check = D.iconButton("check-square", done ? t("标为未完成") : t("标为已完成"), () => toggleTodo(row));
    return D.el("div.dash-todo-row" + (done ? ".is-done" : ""), null,
      check,
      D.el("span.dash-todo-content", { text: row.content, title: row.content }),
      D.el("span.dash-chip" + (row.priority === "high" ? ".is-danger" : row.priority === "low" ? "" : ".is-warn"),
        { text: PRIORITY_LABELS[row.priority] || row.priority }),
      done && row.completed_at ? D.el("small", { text: D.formatTime(row.completed_at) }) : null,
      D.iconButton("trash-2", t("删除"), () => deleteTodo(row)));
  }

  function renderTodos() {
    ui.todoList.textContent = "";
    const open = state.todos.filter((row) => row.status !== "done");
    const done = state.todos.filter((row) => row.status === "done");
    for (const row of open) ui.todoList.append(todoRow(row));
    for (const row of done) ui.todoList.append(todoRow(row));
    if (!state.todos.length) ui.todoList.append(D.el("p.dash-empty", { text: t("没有待办。这里记的是 daemon 级轻量待办,与会话内 todo 工具互不相干。") }));
  }

  /* ── 挂载 ─────────────────────────────────────────────── */

  function mount(root) {
    root.textContent = "";
    const refreshButton = D.el("button.dash-button", { type: "button", onclick: () => refresh() });
    refreshButton.append(D.icon("refresh-cw"), t("刷新"));
    const head = D.el("div.con-head", null,
      D.el("h2", { text: t("目标") }),
      D.el("div.con-head-actions", null, refreshButton));

    ui.goalList = D.el("div.dash-goal-list");
    const goalsCard = D.el("div.dash-card-block", null,
      D.el("h3", { text: t("跨会话目标") }),
      ui.goalList);

    ui.todoInput = D.el("input.dash-todo-input", { type: "text", placeholder: t("记一件要做的事…") });
    ui.todoInput.addEventListener("keydown", (event) => { if (event.key === "Enter") addTodo(); });
    ui.todoPriority = D.el("select.dash-todo-priority", null,
      PRIORITIES.map((value) => D.el("option", { value, text: PRIORITY_LABELS[value] })));
    ui.todoPriority.value = "medium";
    const add = D.el("button.dash-button.is-primary", { type: "button", onclick: () => addTodo() });
    add.append(D.icon("plus"), t("添加"));
    ui.todoList = D.el("div.dash-todo-list");
    const todosCard = D.el("div.dash-card-block", null,
      D.el("h3", { text: t("待办") }),
      D.el("div.dash-todo-form", null, ui.todoInput, ui.todoPriority, add),
      ui.todoList);

    root.append(head, D.el("div.dash-goals-layout", null, goalsCard, todosCard));
  }

  async function refresh() {
    try {
      await Promise.all([loadGoals(), loadTodos()]);
    } catch (error) {
      D.toast(t("失败:{message}", { message: error.message }), "error");
    }
    renderGoals();
    renderTodos();
    state.loaded = true;
  }

  D.register({ name: "goals", root: "dashGoalsRoot", mount, refresh });
})();
