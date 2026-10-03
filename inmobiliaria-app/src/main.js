const invoke = window.__TAURI__.core.invoke;

const state = {
  usuarioActual: null,
  usuarios: [],
  propietarios: [],
  inquilinos: [],
  garantes: [],
  inmuebles: [],
  contratos: [],
  pagos: [],
  liquidaciones: [],
};

// ---------- utilidades ----------

const money = (n) => "$ " + Number(n ?? 0).toLocaleString("es-AR", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
const fmtDate = (s) => {
  if (!s) return "-";
  const [y, m, d] = s.split("-");
  return `${d}/${m}/${y}`;
};
const esc = (s) => (s ?? "").toString().replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));

function toast(msg, isError = false) {
  const el = document.getElementById("toast");
  el.textContent = msg;
  el.classList.remove("hidden");
  el.classList.toggle("error", isError);
  clearTimeout(toast._t);
  toast._t = setTimeout(() => el.classList.add("hidden"), 3500);
}

async function call(cmd, args) {
  try {
    return await invoke(cmd, args);
  } catch (e) {
    toast(typeof e === "string" ? e : "Ocurrió un error", true);
    throw e;
  }
}

function byId(list, id) {
  return list.find((x) => x.id === id);
}

// ---------- navegación ----------

document.querySelectorAll(".nav-btn").forEach((btn) => {
  btn.addEventListener("click", () => switchView(btn.dataset.view));
});

function switchView(view) {
  document.querySelectorAll(".nav-btn").forEach((b) => b.classList.toggle("active", b.dataset.view === view));
  document.querySelectorAll(".view").forEach((v) => v.classList.toggle("active", v.id === `view-${view}`));
}

// ---------- modal genérico ----------

const modalOverlay = document.getElementById("modal-overlay");
const modalTitle = document.getElementById("modal-title");
const modalBody = document.getElementById("modal-body");
const modalSave = document.getElementById("modal-save");

function closeModal() {
  modalOverlay.classList.add("hidden");
  modalBody.innerHTML = "";
}
document.getElementById("modal-close").addEventListener("click", closeModal);
document.getElementById("modal-cancel").addEventListener("click", closeModal);

function fieldHtml(f, value) {
  const v = value ?? f.default ?? "";
  if (f.type === "select") {
    const opts = f.options.map((o) => `<option value="${esc(o.value)}" ${String(o.value) === String(v) ? "selected" : ""}>${esc(o.label)}</option>`).join("");
    return `<select id="f-${f.name}">${opts}</select>`;
  }
  if (f.type === "textarea") {
    return `<textarea id="f-${f.name}">${esc(v)}</textarea>`;
  }
  if (f.type === "checkbox-group") {
    const selected = new Set(value ?? []);
    const items = f.options
      .map((o) => `<label><input type="checkbox" value="${o.value}" ${selected.has(o.value) ? "checked" : ""} data-group="${f.name}"/> ${esc(o.label)}</label>`)
      .join("");
    return `<div class="checkbox-list" id="f-${f.name}">${items || '<span class="hint" style="margin:0;">No hay registros disponibles</span>'}</div>`;
  }
  const step = f.type === "number" ? `step="${f.step ?? "any"}"` : "";
  return `<input type="${f.type ?? "text"}" id="f-${f.name}" value="${esc(v)}" ${step} />`;
}

function openModal(title, fields, values, onSave) {
  modalTitle.textContent = title;
  modalBody.innerHTML = `<div class="form-grid">${fields
    .map(
      (f) =>
        `<div class="form-field ${f.full ? "full" : ""}"><label>${esc(f.label)}</label>${fieldHtml(f, values?.[f.name])}${
          f.hint ? `<small class="field-hint">${esc(f.hint)}</small>` : ""
        }</div>`
    )
    .join("")}</div>`;
  modalOverlay.classList.remove("hidden");

  modalSave.onclick = async () => {
    const result = {};
    for (const f of fields) {
      if (f.type === "checkbox-group") {
        const checked = Array.from(document.querySelectorAll(`#f-${f.name} input[type=checkbox]:checked`)).map((c) => Number(c.value));
        result[f.name] = checked;
      } else {
        const el = document.getElementById(`f-${f.name}`);
        let val = el.value;
        if (f.type === "number") val = val === "" ? (f.nullable ? null : 0) : Number(val);
        if (f.numeric && val !== "") val = Number(val);
        if (f.required && (val === "" || val === null)) {
          toast(`El campo "${f.label}" es obligatorio`, true);
          return;
        }
        result[f.name] = val === "" ? null : val;
      }
    }
    try {
      await onSave(result);
      closeModal();
    } catch (e) {
      /* el toast de error ya se mostró en call() */
    }
  };
}

// ---------- modal de impresión ----------

const printOverlay = document.getElementById("print-overlay");
const printBody = document.getElementById("print-body");
document.getElementById("print-close").addEventListener("click", () => printOverlay.classList.add("hidden"));
document.getElementById("print-close-2").addEventListener("click", () => printOverlay.classList.add("hidden"));
document.getElementById("print-now").addEventListener("click", () => window.print());

function openPrint(html) {
  printBody.innerHTML = html;
  printOverlay.classList.remove("hidden");
}

// ---------- carga inicial ----------

async function loadAll() {
  const [usuarios, propietarios, inquilinos, garantes, inmuebles, contratos, pagos, liquidaciones] = await Promise.all([
    call("get_usuarios"),
    call("get_propietarios"),
    call("get_inquilinos"),
    call("get_garantes"),
    call("get_inmuebles"),
    call("get_contratos"),
    call("get_pagos"),
    call("get_liquidaciones"),
  ]);
  Object.assign(state, { usuarios, propietarios, inquilinos, garantes, inmuebles, contratos, pagos, liquidaciones });
  renderUsuarios();
  renderPropietarios();
  renderInquilinos();
  renderGarantes();
  renderInmuebles();
  renderContratos();
  renderPagos();
  renderLiquidaciones();
  await renderDashboard();
}

// ---------- PROPIETARIOS ----------

function propietarioFields() {
  return [
    { name: "nombre", label: "Nombre completo", required: true, full: true },
    { name: "dni_cuit", label: "DNI / CUIT" },
    { name: "fecha_nacimiento", label: "Fecha de nacimiento", type: "date", nullable: true },
    { name: "telefono", label: "Teléfono" },
    { name: "email", label: "Email" },
    { name: "direccion", label: "Dirección", full: true },
    { name: "datos_bancarios", label: "Datos bancarios (CBU/alias)", full: true, type: "textarea" },
    { name: "notas", label: "Notas", full: true, type: "textarea" },
  ];
}

function renderPropietarios() {
  const tbody = document.getElementById("tbl-propietarios");
  if (!state.propietarios.length) {
    tbody.innerHTML = `<tr class="empty-row"><td colspan="6">No hay propietarios registrados</td></tr>`;
    return;
  }
  tbody.innerHTML = state.propietarios
    .map(
      (p) => `<tr>
      <td>${esc(p.nombre)}</td><td>${esc(p.dni_cuit)}</td><td>${esc(p.telefono)}</td><td>${esc(p.email)}</td><td>${esc(p.datos_bancarios)}</td>
      <td class="actions">
        <button class="btn-link" data-action="edit-propietario" data-id="${p.id}">Editar</button>
        <button class="btn-link" data-action="del-propietario" data-id="${p.id}">Eliminar</button>
      </td></tr>`
    )
    .join("");
}

document.getElementById("btn-nuevo-propietario").addEventListener("click", () => {
  openModal("Nuevo propietario", propietarioFields(), {}, async (values) => {
    await call("guardar_propietario", { propietario: { id: null, ...values } });
    toast("Propietario guardado");
    await loadAll();
  });
});

document.getElementById("tbl-propietarios").addEventListener("click", async (e) => {
  const btn = e.target.closest("button");
  if (!btn) return;
  const id = Number(btn.dataset.id);
  if (btn.dataset.action === "edit-propietario") {
    const p = byId(state.propietarios, id);
    openModal("Editar propietario", propietarioFields(), p, async (values) => {
      await call("guardar_propietario", { propietario: { id, ...values } });
      toast("Propietario actualizado");
      await loadAll();
    });
  }
  if (btn.dataset.action === "del-propietario") {
    if (confirm("¿Eliminar este propietario? Esto fallará si tiene inmuebles asociados.")) {
      await call("eliminar_propietario", { id });
      toast("Propietario eliminado");
      await loadAll();
    }
  }
});

// ---------- INQUILINOS ----------

function inquilinoFields() {
  return [
    { name: "nombre", label: "Nombre completo", required: true, full: true },
    { name: "dni_cuit", label: "DNI / CUIT" },
    { name: "fecha_nacimiento", label: "Fecha de nacimiento", type: "date", nullable: true },
    { name: "telefono", label: "Teléfono" },
    { name: "email", label: "Email" },
    { name: "direccion", label: "Dirección", full: true },
    { name: "notas", label: "Notas", full: true, type: "textarea" },
  ];
}

function renderInquilinos() {
  const tbody = document.getElementById("tbl-inquilinos");
  if (!state.inquilinos.length) {
    tbody.innerHTML = `<tr class="empty-row"><td colspan="6">No hay inquilinos registrados</td></tr>`;
    return;
  }
  tbody.innerHTML = state.inquilinos
    .map(
      (p) => `<tr>
      <td>${esc(p.nombre)}</td><td>${esc(p.dni_cuit)}</td><td>${esc(p.telefono)}</td><td>${esc(p.email)}</td><td>${esc(p.direccion)}</td>
      <td class="actions">
        <button class="btn-link" data-action="edit-inquilino" data-id="${p.id}">Editar</button>
        <button class="btn-link" data-action="del-inquilino" data-id="${p.id}">Eliminar</button>
      </td></tr>`
    )
    .join("");
}

document.getElementById("btn-nuevo-inquilino").addEventListener("click", () => {
  openModal("Nuevo inquilino", inquilinoFields(), {}, async (values) => {
    await call("guardar_inquilino", { inquilino: { id: null, ...values } });
    toast("Inquilino guardado");
    await loadAll();
  });
});

document.getElementById("tbl-inquilinos").addEventListener("click", async (e) => {
  const btn = e.target.closest("button");
  if (!btn) return;
  const id = Number(btn.dataset.id);
  if (btn.dataset.action === "edit-inquilino") {
    const p = byId(state.inquilinos, id);
    openModal("Editar inquilino", inquilinoFields(), p, async (values) => {
      await call("guardar_inquilino", { inquilino: { id, ...values } });
      toast("Inquilino actualizado");
      await loadAll();
    });
  }
  if (btn.dataset.action === "del-inquilino") {
    if (confirm("¿Eliminar este inquilino? Esto fallará si tiene contratos asociados.")) {
      await call("eliminar_inquilino", { id });
      toast("Inquilino eliminado");
      await loadAll();
    }
  }
});

// ---------- GARANTES ----------

function garanteFields() {
  return [
    { name: "nombre", label: "Nombre completo", required: true, full: true },
    { name: "dni_cuit", label: "DNI / CUIT" },
    { name: "fecha_nacimiento", label: "Fecha de nacimiento", type: "date", nullable: true },
    { name: "telefono", label: "Teléfono" },
    { name: "email", label: "Email" },
    { name: "direccion", label: "Dirección", full: true },
    { name: "notas", label: "Notas", full: true, type: "textarea" },
  ];
}

function renderGarantes() {
  const tbody = document.getElementById("tbl-garantes");
  if (!state.garantes.length) {
    tbody.innerHTML = `<tr class="empty-row"><td colspan="6">No hay garantes registrados</td></tr>`;
    return;
  }
  tbody.innerHTML = state.garantes
    .map(
      (p) => `<tr>
      <td>${esc(p.nombre)}</td><td>${esc(p.dni_cuit)}</td><td>${esc(p.telefono)}</td><td>${esc(p.email)}</td><td>${esc(p.direccion)}</td>
      <td class="actions">
        <button class="btn-link" data-action="edit-garante" data-id="${p.id}">Editar</button>
        <button class="btn-link" data-action="del-garante" data-id="${p.id}">Eliminar</button>
      </td></tr>`
    )
    .join("");
}

document.getElementById("btn-nuevo-garante").addEventListener("click", () => {
  openModal("Nuevo garante", garanteFields(), {}, async (values) => {
    await call("guardar_garante", { garante: { id: null, ...values } });
    toast("Garante guardado");
    await loadAll();
  });
});

document.getElementById("tbl-garantes").addEventListener("click", async (e) => {
  const btn = e.target.closest("button");
  if (!btn) return;
  const id = Number(btn.dataset.id);
  if (btn.dataset.action === "edit-garante") {
    const p = byId(state.garantes, id);
    openModal("Editar garante", garanteFields(), p, async (values) => {
      await call("guardar_garante", { garante: { id, ...values } });
      toast("Garante actualizado");
      await loadAll();
    });
  }
  if (btn.dataset.action === "del-garante") {
    if (confirm("¿Eliminar este garante? Esto fallará si está asociado a algún contrato.")) {
      await call("eliminar_garante", { id });
      toast("Garante eliminado");
      await loadAll();
    }
  }
});

// ---------- INMUEBLES ----------

function inmuebleFields() {
  return [
    { name: "direccion", label: "Dirección", required: true, full: true },
    {
      name: "propietario_id",
      label: "Propietario",
      type: "select",
      required: true,
      numeric: true,
      options: state.propietarios.map((p) => ({ value: p.id, label: p.nombre })),
    },
    {
      name: "tipo",
      label: "Tipo de inmueble",
      type: "select",
      options: [
        { value: "Departamento", label: "Departamento" },
        { value: "Casa", label: "Casa" },
        { value: "Local comercial", label: "Local comercial" },
        { value: "Oficina", label: "Oficina" },
        { value: "Terreno", label: "Terreno" },
        { value: "Otro", label: "Otro" },
      ],
    },
    { name: "superficie", label: "Superficie (m²)", type: "number", nullable: true },
    { name: "ambientes", label: "Ambientes", type: "number", nullable: true },
    { name: "notas", label: "Notas", full: true, type: "textarea" },
  ];
}

function renderInmuebles() {
  const tbody = document.getElementById("tbl-inmuebles");
  if (!state.inmuebles.length) {
    tbody.innerHTML = `<tr class="empty-row"><td colspan="6">No hay inmuebles registrados</td></tr>`;
    return;
  }
  tbody.innerHTML = state.inmuebles
    .map(
      (i) => `<tr>
      <td>${esc(i.direccion)}</td><td>${esc(i.tipo)}</td><td>${i.superficie ?? "-"}</td><td>${i.ambientes ?? "-"}</td><td>${esc(i.propietario_nombre)}</td>
      <td class="actions">
        <button class="btn-link" data-action="edit-inmueble" data-id="${i.id}">Editar</button>
        <button class="btn-link" data-action="del-inmueble" data-id="${i.id}">Eliminar</button>
      </td></tr>`
    )
    .join("");
}

document.getElementById("btn-nuevo-inmueble").addEventListener("click", () => {
  if (!state.propietarios.length) {
    toast("Primero registrá al menos un propietario", true);
    return;
  }
  openModal("Nuevo inmueble", inmuebleFields(), {}, async (values) => {
    await call("guardar_inmueble", { inmueble: { id: null, ...values } });
    toast("Inmueble guardado");
    await loadAll();
  });
});

document.getElementById("tbl-inmuebles").addEventListener("click", async (e) => {
  const btn = e.target.closest("button");
  if (!btn) return;
  const id = Number(btn.dataset.id);
  if (btn.dataset.action === "edit-inmueble") {
    const i = byId(state.inmuebles, id);
    openModal("Editar inmueble", inmuebleFields(), i, async (values) => {
      await call("guardar_inmueble", { inmueble: { id, ...values } });
      toast("Inmueble actualizado");
      await loadAll();
    });
  }
  if (btn.dataset.action === "del-inmueble") {
    if (confirm("¿Eliminar este inmueble? Esto fallará si tiene contratos asociados.")) {
      await call("eliminar_inmueble", { id });
      toast("Inmueble eliminado");
      await loadAll();
    }
  }
});

// ---------- CONTRATOS ----------

function contratoFields() {
  return [
    {
      name: "inmueble_id",
      label: "Inmueble",
      type: "select",
      required: true,
      numeric: true,
      options: state.inmuebles.map((i) => ({ value: i.id, label: `${i.direccion} (${i.propietario_nombre})` })),
    },
    {
      name: "inquilino_id",
      label: "Inquilino",
      type: "select",
      required: true,
      numeric: true,
      options: state.inquilinos.map((i) => ({ value: i.id, label: i.nombre })),
    },
    { name: "fecha_inicio", label: "Fecha de inicio", type: "date", required: true },
    { name: "fecha_fin", label: "Fecha de vencimiento", type: "date", required: true },
    {
      name: "dia_pago",
      label: "Paga sin recargo del 1 al día (1-28)",
      type: "number",
      default: 10,
      required: true,
      hint: "Ej: si ponés 10, el inquilino paga sin interés del 1 al 10. Desde el día 11 corre la mora que indica el contrato.",
    },
    { name: "monto_inicial", label: "Monto de alquiler inicial", type: "number", required: true },
    { name: "comision_porcentaje", label: "Comisión inmobiliaria (%)", type: "number", default: 0 },
    { name: "tasa_mora_diaria", label: "Interés por mora (% diario)", type: "number", default: 0 },
    { name: "frecuencia_actualizacion_meses", label: "Frecuencia de actualización (meses)", type: "number", default: 12 },
    {
      name: "tipo_actualizacion",
      label: "Tipo de actualización",
      type: "select",
      options: [
        { value: "porcentaje_fijo", label: "Porcentaje fijo" },
        { value: "ICL", label: "Índice ICL (BCRA)" },
        { value: "IPC", label: "Índice IPC (INDEC)" },
        { value: "casa_propia", label: "Índice Casa Propia" },
        { value: "otro", label: "Otro / a convenir" },
      ],
    },
    { name: "porcentaje_actualizacion", label: "% fijo de actualización (si aplica)", type: "number", default: 0 },
    {
      name: "estado",
      label: "Estado",
      type: "select",
      options: [
        { value: "activo", label: "Activo" },
        { value: "finalizado", label: "Finalizado" },
        { value: "rescindido", label: "Rescindido" },
      ],
    },
    {
      name: "garante_ids",
      label: "Garantes",
      type: "checkbox-group",
      full: true,
      options: state.garantes.map((g) => ({ value: g.id, label: g.nombre })),
    },
    { name: "notas", label: "Notas", full: true, type: "textarea" },
  ];
}

function pillEstado(estado) {
  const cls = estado === "activo" ? "pill-activo" : "pill-finalizado";
  return `<span class="pill ${cls}">${esc(estado)}</span>`;
}

function renderContratos() {
  const tbody = document.getElementById("tbl-contratos");
  if (!state.contratos.length) {
    tbody.innerHTML = `<tr class="empty-row"><td colspan="7">No hay contratos registrados</td></tr>`;
    return;
  }
  tbody.innerHTML = state.contratos
    .map(
      (c) => `<tr>
      <td>${esc(c.inmueble_direccion)}</td><td>${esc(c.inquilino_nombre)}</td>
      <td>${fmtDate(c.fecha_inicio)} → ${fmtDate(c.fecha_fin)}</td>
      <td>${money(c.monto_vigente)}</td>
      <td>${fmtDate(c.proxima_actualizacion)}</td>
      <td>${pillEstado(c.estado)}</td>
      <td class="actions">
        <button class="btn-link" data-action="ver-actualizaciones" data-id="${c.id}">Actualizaciones</button>
        <button class="btn-link" data-action="edit-contrato" data-id="${c.id}">Editar</button>
        <button class="btn-link" data-action="del-contrato" data-id="${c.id}">Eliminar</button>
      </td></tr>`
    )
    .join("");
}

function contratoToForm(c) {
  if (!c) return {};
  return { ...c, garante_ids: c.garante_ids ?? [] };
}

document.getElementById("btn-nuevo-contrato").addEventListener("click", () => {
  if (!state.inmuebles.length || !state.inquilinos.length) {
    toast("Registrá al menos un inmueble y un inquilino primero", true);
    return;
  }
  openModal("Nuevo contrato", contratoFields(), { dia_pago: 10, tipo_actualizacion: "porcentaje_fijo", estado: "activo" }, async (values) => {
    await call("guardar_contrato", { contrato: { id: null, ...values } });
    toast("Contrato guardado");
    await loadAll();
  });
});

document.getElementById("tbl-contratos").addEventListener("click", async (e) => {
  const btn = e.target.closest("button");
  if (!btn) return;
  const id = Number(btn.dataset.id);
  if (btn.dataset.action === "edit-contrato") {
    const c = byId(state.contratos, id);
    openModal("Editar contrato", contratoFields(), contratoToForm(c), async (values) => {
      await call("guardar_contrato", { contrato: { id, ...values } });
      toast("Contrato actualizado");
      await loadAll();
    });
  }
  if (btn.dataset.action === "del-contrato") {
    if (confirm("¿Eliminar este contrato? Esto fallará si tiene pagos registrados.")) {
      await call("eliminar_contrato", { id });
      toast("Contrato eliminado");
      await loadAll();
    }
  }
  if (btn.dataset.action === "ver-actualizaciones") {
    await abrirActualizaciones(id);
  }
});

async function abrirActualizaciones(contratoId) {
  const c = byId(state.contratos, contratoId);
  const actualizaciones = await call("get_actualizaciones", { contratoId });
  const esIcl = c.tipo_actualizacion === "ICL";
  const render = () => {
    const filas = actualizaciones
      .map(
        (a) => `<tr><td>${fmtDate(a.fecha_vigencia)}</td><td>${money(a.monto_nuevo)}</td><td>${esc(a.motivo)}</td>
        <td class="actions"><button class="btn-link" data-del-act="${a.id}">Eliminar</button></td></tr>`
      )
      .join("");
    modalBody.innerHTML = `
      <p class="hint" style="margin-top:0">Monto inicial del contrato: <strong>${money(c.monto_inicial)}</strong> — Monto vigente hoy: <strong>${money(c.monto_vigente)}</strong></p>
      <table class="data-table" style="margin-bottom:16px;">
        <thead><tr><th>Vigente desde</th><th>Nuevo monto</th><th>Motivo</th><th></th></tr></thead>
        <tbody>${filas || '<tr class="empty-row"><td colspan="4">Sin actualizaciones registradas</td></tr>'}</tbody>
      </table>
      ${
        esIcl
          ? `<div style="margin-bottom:18px; padding:12px; border:1px solid var(--border); border-radius:8px; background:#f9fafb;">
               <button class="btn-secondary" id="act-calcular-icl" type="button">📊 Calcular con ICL (BCRA)</button>
               <div id="act-icl-resultado" class="hint" style="margin:8px 0 0;">Próxima actualización prevista: ${fmtDate(c.proxima_actualizacion)}.</div>
             </div>`
          : ""
      }
      <p class="hint" style="margin:0 0 10px;">También podés escribir el % de aumento y calculamos el monto nuevo solos.</p>
      <div class="form-grid">
        <div class="form-field"><label>Vigente desde</label><input type="date" id="act-fecha" value="${esc(c.proxima_actualizacion ?? "")}" /></div>
        <div class="form-field"><label>Aumento (%)</label><input type="number" id="act-porcentaje" step="any" placeholder="ej: 25" /></div>
        <div class="form-field"><label>Nuevo monto de alquiler</label><input type="number" id="act-monto" /></div>
        <div class="form-field full"><label>Motivo (ej: actualización ICL trimestral)</label><input type="text" id="act-motivo" /></div>
      </div>`;

    document.getElementById("act-porcentaje").addEventListener("input", (e) => {
      const pct = Number(e.target.value);
      if (e.target.value === "" || Number.isNaN(pct)) return;
      document.getElementById("act-monto").value = (c.monto_vigente * (1 + pct / 100)).toFixed(2);
    });

    if (esIcl) {
      document.getElementById("act-calcular-icl").addEventListener("click", async () => {
        const resultadoEl = document.getElementById("act-icl-resultado");
        resultadoEl.textContent = "Consultando ICL...";
        const hoy = new Date().toISOString().slice(0, 10);
        const esFutura = c.proxima_actualizacion > hoy;
        try {
          const r = await call(esFutura ? "estimar_actualizacion_icl" : "confirmar_actualizacion_icl", { contratoId });
          const signo = r.porcentaje_variacion >= 0 ? "+" : "";
          if (esFutura) {
            resultadoEl.innerHTML = `Estimado orientativo (el valor real se confirma el ${fmtDate(
              c.proxima_actualizacion
            )}): ICL del ${fmtDate(r.fecha_consulta)} = ${r.valor_icl_consulta} → variación ${signo}${r.porcentaje_variacion.toFixed(
              2
            )}% → alquiler aprox. <strong>${money(r.monto_estimado)}</strong>. Todavía no lo cargues: volvé a calcular el día de la actualización para traer el valor real.`;
          } else {
            document.getElementById("act-fecha").value = c.proxima_actualizacion;
            document.getElementById("act-monto").value = r.monto_estimado.toFixed(2);
            document.getElementById("act-porcentaje").value = r.porcentaje_variacion.toFixed(2);
            document.getElementById("act-motivo").value = "Actualización por índice ICL (BCRA)";
            resultadoEl.innerHTML = `Valor real: ICL del ${fmtDate(r.fecha_consulta)} = ${r.valor_icl_consulta} → variación ${signo}${r.porcentaje_variacion.toFixed(
              2
            )}%. Ya completamos los campos — revisá y confirmá con "Agregar actualización".`;
          }
        } catch (err) {
          resultadoEl.textContent = "No se pudo consultar el ICL. Probá de nuevo.";
        }
      });
    }

    modalBody.querySelectorAll("[data-del-act]").forEach((b) =>
      b.addEventListener("click", async () => {
        await call("eliminar_actualizacion", { id: Number(b.dataset.delAct) });
        const idx = actualizaciones.findIndex((a) => a.id === Number(b.dataset.delAct));
        if (idx >= 0) actualizaciones.splice(idx, 1);
        render();
      })
    );
  };
  modalTitle.textContent = `Actualizaciones — ${c.inmueble_direccion}`;
  render();
  modalOverlay.classList.remove("hidden");
  modalSave.textContent = "Agregar actualización";
  modalSave.onclick = async () => {
    const fecha_vigencia = document.getElementById("act-fecha").value;
    const monto_nuevo = Number(document.getElementById("act-monto").value);
    const motivo = document.getElementById("act-motivo").value || null;
    if (!fecha_vigencia || !monto_nuevo) {
      toast("Completá la fecha y el nuevo monto", true);
      return;
    }
    await call("agregar_actualizacion", { actualizacion: { id: null, contrato_id: contratoId, fecha_vigencia, monto_nuevo, motivo } });
    closeModal();
    modalSave.textContent = "Guardar";
    await loadAll();
  };
}

// ---------- INGRESOS (pagos / recibos) ----------

function pagoFields() {
  return [
    {
      name: "contrato_id",
      label: "Contrato",
      type: "select",
      required: true,
      numeric: true,
      options: state.contratos
        .filter((c) => c.estado === "activo")
        .map((c) => ({ value: c.id, label: `${c.inmueble_direccion} — ${c.inquilino_nombre}` })),
    },
    { name: "periodo", label: "Período que abona (mes/año)", type: "month", required: true },
    { name: "fecha_pago", label: "Fecha de pago", type: "date", required: true, default: new Date().toISOString().slice(0, 10) },
    {
      name: "metodo_pago",
      label: "Método de pago",
      type: "select",
      options: [
        { value: "Efectivo", label: "Efectivo" },
        { value: "Transferencia", label: "Transferencia bancaria" },
        { value: "Débito/Crédito", label: "Débito/Crédito" },
        { value: "Otro", label: "Otro" },
      ],
    },
    { name: "notas", label: "Notas", full: true, type: "textarea" },
  ];
}

function renderPagos() {
  const tbody = document.getElementById("tbl-pagos");
  if (!state.pagos.length) {
    tbody.innerHTML = `<tr class="empty-row"><td colspan="8">No hay pagos registrados</td></tr>`;
    return;
  }
  tbody.innerHTML = state.pagos
    .map((p) => {
      const c = byId(state.contratos, p.contrato_id);
      const lugar = c ? `${c.inmueble_direccion} / ${c.inquilino_nombre}` : "(contrato eliminado)";
      return `<tr>
      <td>${p.numero_recibo}</td><td>${fmtDate(p.fecha_pago)}</td><td>${esc(lugar)}</td><td>${esc(p.periodo)}</td>
      <td>${money(p.monto_alquiler)}</td><td>${p.dias_mora > 0 ? `${money(p.monto_mora)} (${p.dias_mora}d)` : "-"}</td>
      <td><strong>${money(p.monto_total)}</strong></td>
      <td class="actions">
        <button class="btn-link" data-action="ver-recibo" data-id="${p.id}">Ver recibo</button>
        <button class="btn-link" data-action="del-pago" data-id="${p.id}">Eliminar</button>
      </td></tr>`;
    })
    .join("");
}

document.getElementById("btn-nuevo-pago").addEventListener("click", () => {
  if (!state.contratos.some((c) => c.estado === "activo")) {
    toast("No hay contratos activos para registrar un pago", true);
    return;
  }
  const hoy = new Date().toISOString().slice(0, 10);
  openModal("Registrar pago de alquiler", pagoFields(), { fecha_pago: hoy, periodo: hoy.slice(0, 7) }, async (values) => {
    const id = await call("registrar_pago", { nuevo: values });
    toast("Pago registrado — Recibo generado");
    await loadAll();
    await verRecibo(id);
  });
});

document.getElementById("tbl-pagos").addEventListener("click", async (e) => {
  const btn = e.target.closest("button");
  if (!btn) return;
  const id = Number(btn.dataset.id);
  if (btn.dataset.action === "ver-recibo") await verRecibo(id);
  if (btn.dataset.action === "del-pago") {
    if (confirm("¿Eliminar este pago? También se eliminará su comprobante de liquidación si existe.")) {
      await call("eliminar_pago", { id });
      toast("Pago eliminado");
      await loadAll();
    }
  }
});

async function verRecibo(pagoId) {
  const r = await call("get_recibo", { pagoId });
  const p = r.pago;
  openPrint(`
    <div class="recibo">
      <div class="recibo-header">
        <div>
          <h2>Recibo de Alquiler</h2>
          <div>Inmobiliaria App</div>
        </div>
        <div class="recibo-numero">Recibo N°<strong>${String(p.numero_recibo).padStart(6, "0")}</strong>Fecha: ${fmtDate(p.fecha_pago)}</div>
      </div>
      <div class="recibo-grid">
        <div><span>Inquilino</span><strong>${esc(r.inquilino_nombre)}</strong></div>
        <div><span>Propietario</span><strong>${esc(r.propietario_nombre)}</strong></div>
        <div><span>Inmueble</span><strong>${esc(r.inmueble_direccion)}</strong></div>
        <div><span>Período abonado</span><strong>${esc(p.periodo)}</strong></div>
        <div><span>Método de pago</span><strong>${esc(p.metodo_pago ?? "-")}</strong></div>
        <div><span>Días de mora</span><strong>${p.dias_mora}</strong></div>
      </div>
      <div class="recibo-totales">
        <table>
          <tr><td>Monto de alquiler</td><td style="text-align:right">${money(p.monto_alquiler)}</td></tr>
          <tr><td>Interés por mora</td><td style="text-align:right">${money(p.monto_mora)}</td></tr>
          <tr class="total-row"><td>Total abonado</td><td style="text-align:right">${money(p.monto_total)}</td></tr>
        </table>
      </div>
      ${p.notas ? `<p class="hint">Notas: ${esc(p.notas)}</p>` : ""}
      <div class="recibo-firma">
        <div>Firma del locador / inmobiliaria</div>
        <div>Firma del inquilino</div>
      </div>
    </div>`);
}

// ---------- EGRESOS (liquidaciones a propietarios) ----------

function renderLiquidaciones() {
  const tbody = document.getElementById("tbl-liquidaciones");
  const btnHeader = document.querySelector("#view-egresos .view-header");
  if (!btnHeader.querySelector("#btn-nueva-liquidacion")) {
    const btn = document.createElement("button");
    btn.className = "btn-primary";
    btn.id = "btn-nueva-liquidacion";
    btn.textContent = "+ Generar comprobante";
    btn.addEventListener("click", abrirNuevaLiquidacion);
    btnHeader.appendChild(btn);
  }
  if (!state.liquidaciones.length) {
    tbody.innerHTML = `<tr class="empty-row"><td colspan="7">No hay comprobantes generados</td></tr>`;
    return;
  }
  tbody.innerHTML = state.liquidaciones
    .map((l) => {
      const pago = byId(state.pagos, l.pago_id);
      const c = pago ? byId(state.contratos, pago.contrato_id) : null;
      const prop = c ? c.propietario_nombre : "-";
      return `<tr>
      <td>${l.numero_comprobante}</td><td>${fmtDate(l.fecha)}</td><td>${esc(prop)}</td>
      <td>${money(l.monto_alquiler)}</td><td>${money(l.monto_comision)} (${l.comision_porcentaje}%)</td>
      <td><strong>${money(l.monto_neto)}</strong></td>
      <td class="actions">
        <button class="btn-link" data-action="ver-comprobante" data-id="${l.id}">Ver comprobante</button>
        <button class="btn-link" data-action="del-liquidacion" data-id="${l.id}">Eliminar</button>
      </td></tr>`;
    })
    .join("");
}

document.getElementById("tbl-liquidaciones").addEventListener("click", async (e) => {
  const btn = e.target.closest("button");
  if (!btn) return;
  const id = Number(btn.dataset.id);
  if (btn.dataset.action === "ver-comprobante") await verComprobante(id);
  if (btn.dataset.action === "del-liquidacion") {
    if (confirm("¿Eliminar este comprobante de liquidación?")) {
      await call("eliminar_liquidacion", { id });
      toast("Comprobante eliminado");
      await loadAll();
    }
  }
});

function abrirNuevaLiquidacion() {
  const pagosConLiquidacion = new Set(state.liquidaciones.map((l) => l.pago_id));
  const disponibles = state.pagos.filter((p) => !pagosConLiquidacion.has(p.id));
  if (!disponibles.length) {
    toast("No hay recibos de alquiler pendientes de liquidar", true);
    return;
  }
  const fields = [
    {
      name: "pago_id",
      label: "Recibo de alquiler cobrado",
      type: "select",
      required: true,
      options: disponibles.map((p) => {
        const c = byId(state.contratos, p.contrato_id);
        return { value: p.id, label: `Recibo N°${p.numero_recibo} — ${c ? c.inmueble_direccion : ""} (${p.periodo}) — ${money(p.monto_total)}` };
      }),
    },
    { name: "comision_porcentaje", label: "Comisión a aplicar (%) — vacío = usar la del contrato", type: "number", nullable: true },
    { name: "fecha", label: "Fecha del comprobante", type: "date", required: true, default: new Date().toISOString().slice(0, 10) },
    { name: "notas", label: "Notas", full: true, type: "textarea" },
  ];
  openModal("Generar comprobante de liquidación", fields, { fecha: new Date().toISOString().slice(0, 10) }, async (values) => {
    const id = await call("generar_liquidacion", {
      pagoId: Number(values.pago_id),
      comisionPorcentaje: values.comision_porcentaje === null ? null : Number(values.comision_porcentaje),
      fecha: values.fecha,
      notas: values.notas,
    });
    toast("Comprobante generado");
    await loadAll();
    await verComprobante(id);
  });
}

async function verComprobante(liquidacionId) {
  const c = await call("get_comprobante", { liquidacionId });
  const l = c.liquidacion;
  openPrint(`
    <div class="recibo">
      <div class="recibo-header">
        <div>
          <h2>Liquidación al Propietario</h2>
          <div>Inmobiliaria App — Comprobante de comisión</div>
        </div>
        <div class="recibo-numero">Comprobante N°<strong>${String(l.numero_comprobante).padStart(6, "0")}</strong>Fecha: ${fmtDate(l.fecha)}</div>
      </div>
      <div class="recibo-grid">
        <div><span>Propietario</span><strong>${esc(c.propietario_nombre)}</strong></div>
        <div><span>Inquilino</span><strong>${esc(c.inquilino_nombre)}</strong></div>
        <div><span>Inmueble</span><strong>${esc(c.inmueble_direccion)}</strong></div>
        <div><span>Período liquidado</span><strong>${esc(c.periodo)}</strong></div>
        <div class="full" style="grid-column:1/-1"><span>Datos bancarios para transferencia</span><strong>${esc(c.propietario_datos_bancarios || "No registrados")}</strong></div>
      </div>
      <div class="recibo-totales">
        <table>
          <tr><td>Monto de alquiler cobrado</td><td style="text-align:right">${money(l.monto_alquiler)}</td></tr>
          <tr><td>Comisión inmobiliaria (${l.comision_porcentaje}%)</td><td style="text-align:right">- ${money(l.monto_comision)}</td></tr>
          <tr class="total-row"><td>Monto neto a transferir</td><td style="text-align:right">${money(l.monto_neto)}</td></tr>
        </table>
      </div>
      ${l.notas ? `<p class="hint">Notas: ${esc(l.notas)}</p>` : ""}
      <div class="recibo-firma">
        <div>Firma inmobiliaria</div>
        <div>Recibí conforme — Propietario</div>
      </div>
    </div>`);
}

// ---------- DASHBOARD ----------

async function renderDashboard() {
  const dias_vencimiento = Number(document.getElementById("dash-dias-venc").value || 60);
  const dias_actualizacion = Number(document.getElementById("dash-dias-act").value || 30);
  const dias_cumpleanos = Number(document.getElementById("dash-dias-cumple").value || 30);
  const d = await call("get_dashboard", { diasVencimiento: dias_vencimiento, diasActualizacion: dias_actualizacion, diasCumpleanos: dias_cumpleanos });

  document.getElementById("dash-contratos-activos").textContent = d.total_contratos_activos;
  document.getElementById("dash-total-adeudado").textContent = money(d.total_adeudado);
  document.getElementById("dash-cant-deudas").textContent = d.deudas.length;
  document.getElementById("dash-cant-vencimientos").textContent = d.vencimientos.length;

  const tblDeudas = document.getElementById("tbl-deudas");
  tblDeudas.innerHTML = d.deudas.length
    ? d.deudas
        .map(
          (x) => `<tr><td>${esc(x.inmueble_direccion)}</td><td>${esc(x.inquilino_nombre)}</td>
          <td>${x.periodos_impagos.map(esc).join(", ")}</td><td><strong>${money(x.monto_adeudado)}</strong></td></tr>`
        )
        .join("")
    : `<tr class="empty-row"><td colspan="4">No hay deudas pendientes 🎉</td></tr>`;

  const tblVenc = document.getElementById("tbl-vencimientos");
  tblVenc.innerHTML = d.vencimientos.length
    ? d.vencimientos
        .map((x) => {
          const pill = x.dias_restantes < 0 ? "pill-danger" : x.dias_restantes <= 15 ? "pill-warning" : "pill-activo";
          const txt = x.dias_restantes < 0 ? `Vencido hace ${Math.abs(x.dias_restantes)} días` : `${x.dias_restantes} días`;
          return `<tr><td>${esc(x.inmueble_direccion)}</td><td>${esc(x.inquilino_nombre)}</td><td>${esc(x.propietario_nombre)}</td>
          <td>${fmtDate(x.fecha_fin)}</td><td><span class="pill ${pill}">${txt}</span></td></tr>`;
        })
        .join("")
    : `<tr class="empty-row"><td colspan="5">Sin vencimientos próximos</td></tr>`;

  const tblAct = document.getElementById("tbl-actualizaciones");
  tblAct.innerHTML = d.actualizaciones_pendientes.length
    ? d.actualizaciones_pendientes
        .map((x) => {
          const pill = x.dias_restantes < 0 ? "pill-danger" : "pill-warning";
          const txt = x.dias_restantes < 0 ? `Atrasada ${Math.abs(x.dias_restantes)} días` : `${x.dias_restantes} días`;
          let celdaIndice;
          if (x.tipo_actualizacion === "ICL") {
            celdaIndice =
              x.dias_restantes > 0
                ? `<button class="btn-link" data-action="icl-estimar" data-id="${x.contrato_id}">🔄 Estimar con ICL</button>`
                : `<button class="btn-link" data-action="icl-real" data-id="${x.contrato_id}" data-fecha="${x.fecha_prevista}">🔄 Traer valor real ICL</button>`;
          } else {
            celdaIndice = esc(x.tipo_actualizacion || "-");
          }
          return `<tr><td>${esc(x.inmueble_direccion)}</td><td>${esc(x.inquilino_nombre)}</td><td>${fmtDate(x.fecha_prevista)}</td>
          <td><span class="pill ${pill}">${txt}</span></td><td>${money(x.monto_vigente)}</td><td>${celdaIndice}</td></tr>`;
        })
        .join("")
    : `<tr class="empty-row"><td colspan="6">Sin actualizaciones próximas</td></tr>`;

  const tblCumple = document.getElementById("tbl-cumpleanos");
  tblCumple.innerHTML = d.cumpleanos_proximos.length
    ? d.cumpleanos_proximos
        .map((x) => {
          const pill = x.dias_restantes === 0 ? "pill-warning" : "pill-activo";
          const txt = x.dias_restantes === 0 ? "¡Es hoy!" : `${x.dias_restantes} días`;
          return `<tr><td>${esc(x.nombre)}</td><td>${esc(x.tipo)}</td><td>${fmtDate(x.proximo_cumple)}</td>
          <td>${x.edad_cumple} años</td><td><span class="pill ${pill}">${txt}</span></td></tr>`;
        })
        .join("")
    : `<tr class="empty-row"><td colspan="5">Sin cumpleaños próximos</td></tr>`;
}

document.getElementById("dash-dias-venc").addEventListener("change", renderDashboard);
document.getElementById("dash-dias-act").addEventListener("change", renderDashboard);
document.getElementById("dash-dias-cumple").addEventListener("change", renderDashboard);

// ---------- actualización de alquiler vía índice ICL (BCRA) ----------

function resumenIcl(r) {
  const signo = r.porcentaje_variacion >= 0 ? "+" : "";
  return `${money(r.monto_estimado)} <span class="hint" style="margin:0">(${signo}${r.porcentaje_variacion.toFixed(2)}% · ICL ${fmtDate(r.fecha_consulta)}: ${r.valor_icl_consulta})</span>`;
}

document.getElementById("tbl-actualizaciones").addEventListener("click", async (e) => {
  const btn = e.target.closest("button");
  if (!btn) return;
  const contratoId = Number(btn.dataset.id);
  const celda = btn.closest("td");

  if (btn.dataset.action === "icl-estimar") {
    celda.textContent = "Consultando ICL...";
    try {
      const r = await call("estimar_actualizacion_icl", { contratoId });
      celda.innerHTML = `${resumenIcl(r)}<br><small class="hint" style="margin:0">Aproximado — el valor real se confirma el día de la actualización</small>`;
    } catch (err) {
      celda.innerHTML = `<button class="btn-link" data-action="icl-estimar" data-id="${contratoId}">🔄 Reintentar</button>`;
    }
  }

  if (btn.dataset.action === "icl-real") {
    const fechaVigencia = btn.dataset.fecha;
    celda.textContent = "Consultando ICL...";
    try {
      const r = await call("confirmar_actualizacion_icl", { contratoId });
      celda.innerHTML = `${resumenIcl(r)}<br><button class="btn-link" data-action="icl-aplicar" data-id="${contratoId}" data-monto="${r.monto_estimado}" data-fecha="${fechaVigencia}">✅ Aplicar actualización</button>`;
    } catch (err) {
      celda.innerHTML = `<button class="btn-link" data-action="icl-real" data-id="${contratoId}" data-fecha="${fechaVigencia}">🔄 Reintentar</button>`;
    }
  }

  if (btn.dataset.action === "icl-aplicar") {
    const monto_nuevo = Number(btn.dataset.monto);
    const fecha_vigencia = btn.dataset.fecha;
    await call("agregar_actualizacion", {
      actualizacion: { id: null, contrato_id: contratoId, fecha_vigencia, monto_nuevo, motivo: "Actualización por índice ICL (BCRA)" },
    });
    toast("Actualización aplicada según ICL");
    await loadAll();
  }
});

// ---------- USUARIOS ----------

function renderUsuarios() {
  const tbody = document.getElementById("tbl-usuarios");
  if (!state.usuarios.length) {
    tbody.innerHTML = `<tr class="empty-row"><td colspan="4">No hay usuarios</td></tr>`;
    return;
  }
  tbody.innerHTML = state.usuarios
    .map(
      (u) => `<tr>
      <td>${esc(u.username)}</td><td>${esc(u.nombre_completo)}</td>
      <td><span class="pill ${u.activo ? "pill-activo" : "pill-finalizado"}">${u.activo ? "Activo" : "Deshabilitado"}</span></td>
      <td class="actions">
        ${u.id === state.usuarioActual?.id ? "" : `<button class="btn-link" data-action="del-usuario" data-id="${u.id}">Eliminar</button>`}
      </td></tr>`
    )
    .join("");
}

document.getElementById("btn-nuevo-usuario").addEventListener("click", () => {
  openModal(
    "Nuevo usuario",
    [
      { name: "nombre_completo", label: "Nombre completo", required: true, full: true },
      { name: "username", label: "Usuario", required: true, full: true },
      { name: "password", label: "Contraseña (mínimo 4 caracteres)", type: "password", required: true, full: true },
    ],
    {},
    async (values) => {
      await call("crear_usuario", { nuevo: values });
      toast("Usuario creado");
      await loadAll();
    }
  );
});

document.getElementById("tbl-usuarios").addEventListener("click", async (e) => {
  const btn = e.target.closest("button");
  if (!btn) return;
  if (btn.dataset.action === "del-usuario") {
    if (confirm("¿Eliminar este usuario? Ya no va a poder iniciar sesión.")) {
      await call("eliminar_usuario", { id: Number(btn.dataset.id) });
      toast("Usuario eliminado");
      await loadAll();
    }
  }
});

document.getElementById("btn-logout").addEventListener("click", () => {
  state.usuarioActual = null;
  document.getElementById("app").classList.add("hidden");
  mostrarLogin();
});

// ---------- conexión a la base de datos ----------

const conexionScreen = document.getElementById("conexion-screen");
const conexionForm = document.getElementById("conexion-form");
const conexionError = document.getElementById("conexion-error");

conexionForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  conexionError.textContent = "";
  const boton = document.getElementById("conexion-submit");
  boton.disabled = true;
  boton.textContent = "Conectando...";
  const url = document.getElementById("conexion-url").value;
  const anonKey = document.getElementById("conexion-anon-key").value;
  try {
    await invoke("configurar_conexion", { url, anonKey });
    conexionScreen.classList.add("hidden");
    await mostrarLogin();
  } catch (err) {
    conexionError.textContent = typeof err === "string" ? err : "No se pudo conectar";
  } finally {
    boton.disabled = false;
    boton.textContent = "Conectar";
  }
});

// ---------- login ----------

const loginScreen = document.getElementById("login-screen");
const loginForm = document.getElementById("login-form");
const loginError = document.getElementById("login-error");

async function mostrarLogin(usernamePrecargado) {
  loginError.textContent = "";
  loginForm.reset();
  const esPrimerUso = !(await call("hay_usuarios"));
  document.getElementById("login-field-nombre").style.display = esPrimerUso ? "flex" : "none";
  document.getElementById("login-nombre").required = esPrimerUso;
  document.getElementById("login-subtitle").textContent = esPrimerUso
    ? "Todavía no hay usuarios cargados — creá el primero para empezar"
    : "Ingresá tu usuario y contraseña";
  document.getElementById("login-submit").textContent = esPrimerUso ? "Crear usuario y entrar" : "Ingresar";
  loginForm.dataset.modo = esPrimerUso ? "crear" : "login";
  if (usernamePrecargado) document.getElementById("login-username").value = usernamePrecargado;
  loginScreen.classList.remove("hidden");
}

function aplicarUsuarioActual(usuario) {
  state.usuarioActual = usuario;
  document.getElementById("user-name").textContent = usuario.nombre_completo;
  document.getElementById("user-avatar").textContent = usuario.nombre_completo.trim().charAt(0).toUpperCase() || "?";
}

async function entrarAlApp(usuario) {
  aplicarUsuarioActual(usuario);
  loginScreen.classList.add("hidden");
  document.getElementById("app").classList.remove("hidden");
  await loadAll();
  try {
    await invoke("maximizar_ventana");
  } catch (err) {
    console.error(err);
  }
}

loginForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  loginError.textContent = "";
  const username = document.getElementById("login-username").value;
  const password = document.getElementById("login-password").value;
  try {
    let usuario;
    if (loginForm.dataset.modo === "crear") {
      const nombre_completo = document.getElementById("login-nombre").value;
      usuario = await invoke("crear_usuario", { nuevo: { username, password, nombre_completo } });
    } else {
      usuario = await invoke("iniciar_sesion", { username, password });
    }
    try {
      if (document.getElementById("login-recordar").checked) {
        await invoke("guardar_credenciales", { username, password });
      } else {
        await invoke("borrar_credenciales");
      }
    } catch (err) {
      console.error(err);
    }
    await entrarAlApp(usuario);
  } catch (err) {
    loginError.textContent = typeof err === "string" ? err : "No se pudo iniciar sesión";
  }
});

// ---------- arranque ----------

async function arrancar() {
  document.getElementById("app").classList.add("hidden");
  try {
    document.getElementById("app-version").textContent = `v${await invoke("obtener_version")}`;
  } catch (err) {
    console.error(err);
  }
  let conectado;
  try {
    conectado = await invoke("hay_configuracion_conexion");
  } catch (err) {
    conexionError.textContent = typeof err === "string" ? err : "No se pudo conectar con la base guardada";
    conexionScreen.classList.remove("hidden");
    return;
  }
  if (conectado) {
    let recordadas = null;
    try {
      recordadas = await invoke("leer_credenciales_guardadas");
    } catch (err) {
      console.error(err);
    }
    if (recordadas) {
      try {
        const usuario = await invoke("iniciar_sesion", { username: recordadas.username, password: recordadas.password });
        await entrarAlApp(usuario);
        return;
      } catch (err) {
        try {
          await invoke("borrar_credenciales");
        } catch (err2) {
          console.error(err2);
        }
      }
    }
    await mostrarLogin(recordadas ? recordadas.username : undefined);
  } else {
    conexionScreen.classList.remove("hidden");
  }
}

arrancar().catch((e) => console.error(e));
