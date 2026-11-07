/*
 * 技能面板。
 *
 * 数据来自 /api/dash/skills/*,后端走 miyu_core::skills::discover——**与模型
 * 拿到的目录同一条路**,面板上有什么、模型就能用什么。早先只有 miyu skills
 * list(只扫目录、不过白名单)和模型侧 <available-skills> 标签,两边可见性
 * 口径不一样;这个面板按模型视角渲染,把不一致消灭掉。
 *
 * 行点开抽屉:人槽/模型槽的名称与摘要、来源、许可、正文(SKILL.md 全文)。
 * 目录技能(非内置)可以禁用/启用——.disabled 标记文件,与 miyu skills
 * disable 同一套约定;内置技能住资源树里,面板不给写。
 */
(() => {
  const D = window.MiyuDash;
  if (!D) return;

  const state = { overview: null, q: "", filter: "all", loadSeq: 0, bodyCache: {} };
  const ui = {};

  const SOURCE_LABEL = { global: t("全局"), persona: t("人格"), built_in: t("内置") };
  const SOURCE_CLASS = { global: "is-active", persona: "is-warn", built_in: "is-builtin" };

  const chip = (label, cls) => D.el(`span.dash-chip${cls ? `.${cls}` : ""}`, { text: label });
  const rowAttrs = (open) => ({ role: "row", tabindex: "0", onclick: open, onkeydown: (event) => { if (event.key === "Enter") open(); } });

  /* ── 挂载 ─────────────────────────────────────────────── */
  function mount(root) {
    root.textContent = "";
    ui.stamp = D.el("small", { text: "" });
    const head = D.el("div.con-head", null,
      D.el("h2", { text: t("技能") }),
      D.iconButton("refresh-cw", t("刷新"), () => load()),
      ui.stamp);
    ui.cards = D.el("div");
    ui.banner = D.el("div");
    ui.search = D.el("input.dash-search", { type: "search", placeholder: t("按 id、名称或描述筛选…"), oninput: () => {
      clearTimeout(ui.searchTimer);
      ui.searchTimer = setTimeout(() => { state.q = ui.search.value.trim().toLowerCase(); renderList(); }, 250);
    } });
    ui.filter = D.segmented([
      { value: "all", label: t("全部") }, { value: "global", label: t("全局") },
      { value: "persona", label: t("人格") }, { value: "built_in", label: t("内置") }
    ], state.filter, (value) => { state.filter = value; renderList(); });
    const toolbar = D.el("div.dash-toolbar", null, ui.filter.el, D.el("label.dash-search-box", null, D.icon("search"), ui.search));
    ui.list = D.el("div");
    root.append(head, ui.cards, ui.banner, toolbar, ui.list);
    load();
  }

  async function load() {
    const seq = ++state.loadSeq;
    ui.stamp.textContent = t("载入中…");
    state.bodyCache = {};
    try {
      const overview = await D.api("/api/dash/skills/overview");
      if (seq !== state.loadSeq) return;
      state.overview = overview;
      renderCards();
      renderList();
      ui.stamp.textContent = t("{count} 个技能 · 模型视角(与 <available-skills> 同口径)", { count: overview.total });
    } catch (error) {
      ui.stamp.textContent = t("加载失败:{error}", { error: error.message });
    }
  }

  /* ── 统计卡 ───────────────────────────────────────────── */
  function renderCards() {
    const o = state.overview;
    const by = o.by_source || {};
    ui.cards.replaceChildren(D.statCards([
      { label: t("技能总数"), value: o.total, hint: t("模型当前能看到的全部") },
      { label: t("全局"), value: by.global || 0, hint: t("~/.miyu/extensions/skills 下的") },
      { label: t("内置"), value: by.built_in || 0, hint: t("资源树里出厂那份") },
      { label: t("命令执行"), value: o.allow_command_execution ? t("开") : t("关"), hint: t("skills.allow_command_execution") },
    ]));
    ui.banner.textContent = "";
    if (!o.enabled) {
      ui.banner.append(D.el("p.dash-banner", { text: t("skills.enabled 当前是关的:技能目录在,但模型一个都拿不到。到设置页打开后这里才会变绿。") }));
    }
  }

  function visibleSkills() {
    const q = state.q;
    return state.overview.skills.filter((skill) => {
      if (state.filter !== "all" && skill.source !== state.filter) return false;
      if (!q) return true;
      return [skill.id, skill.name, skill.summary, skill.description].some((text) => (text || "").toLowerCase().includes(q));
    });
  }

  function renderList() {
    ui.list.textContent = "";
    const skills = visibleSkills();
    ui.list.append(D.el("h3.dash-section", { text: t("技能({count})", { count: skills.length }) }));
    if (!skills.length) {
      ui.list.append(D.el("p.dash-empty", { text: state.q ? t("没有匹配的技能。") : t("当前视角下没有任何技能。") }));
      return;
    }
    const grid = D.table([
      { label: t("技能"), width: "minmax(220px, 2fr)" }, { label: t("来源"), width: "76px" },
      { label: t("摘要"), width: "minmax(260px, 3fr)" }, { label: t("许可"), width: "90px" }, { label: "", width: "96px" }]);
    for (const skill of skills) grid.append(skillRow(skill));
    ui.list.append(grid);
  }

  function skillRow(skill) {
    const actions = D.el("span.dash-cell-actions", null,
      D.el("button.dash-button", { type: "button", text: t("查看"), title: t("打开正文抽屉"), onclick: (event) => { event.stopPropagation(); openSkill(skill); } }),
      skill.source === "built_in" ? null : D.el("button.dash-button", { type: "button", text: t("禁用"), title: t("写 .disabled 标记,文件保留"), onclick: (event) => { event.stopPropagation(); disable(skill); } }));
    return D.el("div.dash-row", rowAttrs(() => openSkill(skill)),
      D.el("span.dash-cell-main", null,
        D.el("strong", { text: skill.name }),
        D.el("span.dash-cell-muted", { text: ` ${skill.id}` })),
      D.el("span", null, chip(SOURCE_LABEL[skill.source] || skill.source, SOURCE_CLASS[skill.source])),
      D.el("span.dash-cell-muted", { text: skill.summary || skill.description || "—", title: skill.summary || skill.description || "" }),
      D.el("span.dash-cell-mono", { text: skill.license || "—" }),
      actions);
  }

  /* ── 抽屉 ─────────────────────────────────────────────── */
  async function openSkill(skill) {
    let body = state.bodyCache[skill.id];
    if (body === undefined) {
      try {
        const payload = await D.api(`/api/dash/skills/source?id=${encodeURIComponent(skill.id)}`);
        body = payload.missing ? t("(该技能在当前视角下不可见)") : (payload.body || "");
        state.bodyCache[skill.id] = body;
      } catch (error) {
        body = t("正文加载失败:{error}", { error: error.message });
      }
    }
    const pre = D.el("pre.dash-code");
    String(body).split("\n").forEach((line, index) => {
      pre.append(D.el("span.dash-code-line", null, D.el("span.dash-code-no", { text: String(index + 1) }), D.el("span.dash-code-text", { text: line || " " })));
    });
    D.openDrawer(t("技能 {id}", { id: skill.id }), D.el("div", null,
      D.el("dl.dash-meta", null,
        D.el("dt", { text: t("来源") }), D.el("dd", { text: SOURCE_LABEL[skill.source] || skill.source }),
        D.el("dt", { text: t("人槽名称") }), D.el("dd", { text: skill.name }),
        D.el("dt", { text: t("模型触发") }), D.el("dd", { text: skill.description || "—" }),
        skill.compatibility ? (D.el("dt", { text: t("兼容性") }), D.el("dd", { text: skill.compatibility })) : null,
        skill.allowed_tools ? (D.el("dt", { text: t("允许的工具") }), D.el("dd", { text: skill.allowed_tools })) : null,
        skill.path ? (D.el("dt", { text: t("路径") }), D.el("dd", { text: skill.path })) : null),
      pre));
  }

  async function disable(skill) {
    const ok = await D.confirmAction(t("禁用 {id}?写一个 .disabled 标记文件,正文保留,随时可启用。", { id: skill.id }));
    if (!ok) return;
    try {
      await D.api("/api/dash/skills/disable", { method: "POST", body: { id: skill.id } });
      D.toast(t("已禁用 {id}", { id: skill.id }));
      D.closeDrawer();
      await load();
    } catch (error) {
      D.toast(t("禁用失败:{error}", { error: error.message }), "error");
    }
  }

  D.register({ name: "skills", root: "dashSkillsRoot", mount, refresh: () => load() });
})();
