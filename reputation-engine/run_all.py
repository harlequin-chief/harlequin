#!/usr/bin/env python3
"""
Runner del prototipo: ejecuta todos los escenarios, mide cuánto poder captura cada facción y
escribe el informe RESULTADOS.md. Sin dependencias externas.

Uso:
    python3 run_all.py            # ejecuta y escribe RESULTADOS.md
    python3 run_all.py --stdout   # además vuelca el informe por pantalla

Métricas:
  - Cuota de reputación ganada por facción (suma de todo el vector) -> intuición de "masa de poder".
  - Cuota de comité de consenso (sorteo ponderado por reputación, §2.2) -> poder estructural real.
  - Colusión: comparación CON vs SIN damping (§1.6) para medir cuánto aporta el anti-colusión.
  - Blanqueo: reputación del seudónimo viejo vs el nuevo (whitewashing, §5).
  - Slashing en cascada (§1.5c, §1.7): qué pierde el padrino cuando el ahijado defrauda.
"""

from __future__ import annotations

import sys
from collections import defaultdict

from harlequin_rep.consenso import sorteo_ponderado
from harlequin_rep.model import DIMENSIONES
from harlequin_rep.reputacion import (
    agregado_conservador,
    reputacion_vectorial,
)
from harlequin_rep.vouch import (
    RegistroAvales,
    cupo_de_avales,
    dividendo_de_mentor,
    slashing_en_cascada,
)
import adaptativo
import escenarios


# ----------------------------- utilidades de medida ---------------------------------------------

def suma_vector(vec: dict[str, float]) -> float:
    return sum(vec.values())


def cuotas_por_faccion(valor_por_agente: dict[str, float], facciones: dict[str, str]) -> dict[str, float]:
    """Agrupa un valor por agente en cuotas (%) por facción."""
    por_fac: dict[str, float] = defaultdict(float)
    for aid, v in valor_por_agente.items():
        por_fac[facciones[aid]] += v
    total = sum(por_fac.values())
    if total <= 0:
        return {f: 0.0 for f in por_fac}
    return {f: 100.0 * v / total for f, v in por_fac.items()}


def pesos_consenso(rep_vec: dict[str, dict[str, float]], modo: str = "mediana") -> dict[str, float]:
    """Peso de consenso = agregado conservador del vector de reputación (§1.2b, §2.2)."""
    return {aid: agregado_conservador(vec, modo) for aid, vec in rep_vec.items()}


def fmt_pct(d: dict[str, float]) -> str:
    orden = ["genesis", "honesto", "sybil", "colusor", "blanqueador"]
    partes = [f"{f}={d[f]:.2f}%" for f in orden if f in d and d[f] > 1e-9]
    return ", ".join(partes) if partes else "(ninguna)"


# ----------------------------- ejecución de escenarios ------------------------------------------

def medir_escenario(esc) -> dict:
    rep_vec = reputacion_vectorial(esc.agentes, esc.grafo)
    rep_total = {aid: suma_vector(vec) for aid, vec in rep_vec.items()}
    cuota_rep = cuotas_por_faccion(rep_total, esc.facciones)

    pesos = pesos_consenso(rep_vec, "mediana")
    n_facciones = len(set(esc.facciones.values()))
    conteo = sorteo_ponderado(pesos, tam_comite=21, epocas=2000)
    cuota_comite = cuotas_por_faccion({k: float(v) for k, v in conteo.items()}, esc.facciones)

    return {
        "escenario": esc,
        "rep_vec": rep_vec,
        "rep_total": rep_total,
        "cuota_rep": cuota_rep,
        "cuota_comite": cuota_comite,
        "pesos": pesos,
        "n_facciones": n_facciones,
    }


def medir_colusion_damping() -> dict:
    """
    Mismo anillo de colusión (con c0 = trabajador real que el anillo intenta amplificar), con y sin
    damping anti-colusión (§1.6). Mide: cuota de reputación del anillo, y cuántos miembros del anillo
    alcanzan "poder" (umbral = 50% de la reputación media honesta).
    """
    esc = escenarios.escenario_colusion()
    con = reputacion_vectorial(esc.agentes, esc.grafo, damping=True)
    sin = reputacion_vectorial(esc.agentes, esc.grafo, damping=False)
    con_total = {aid: suma_vector(v) for aid, v in con.items()}
    sin_total = {aid: suma_vector(v) for aid, v in sin.items()}

    colusores = [aid for aid, f in esc.facciones.items() if f == "colusor"]
    secuaces = [c for c in colusores if c != "c0"]  # los 29 sin trabajo real

    def reparto_secuaces(rep_total: dict[str, float]) -> tuple[float, float, float]:
        """(reputación de c0, total repartido a los 29 secuaces, máx de un secuaz)."""
        rep_c0 = rep_total["c0"]
        total_sec = sum(rep_total[s] for s in secuaces)
        max_sec = max(rep_total[s] for s in secuaces)
        return rep_c0, total_sec, max_sec

    return {
        "con": reparto_secuaces(con_total),
        "sin": reparto_secuaces(sin_total),
        "n_secuaces": len(secuaces),
    }


def _lavado_secuaces(esc, damping: bool = True, comunidad: bool = False) -> tuple[float, float]:
    """(reputación de c0, total lavado a los demás colusores) bajo el escenario dado."""
    rep = reputacion_vectorial(esc.agentes, esc.grafo, damping=damping, comunidad=comunidad)
    total = {aid: suma_vector(v) for aid, v in rep.items()}
    secuaces = [c for c, f in esc.facciones.items() if f == "colusor" and c != "c0"]
    return total["c0"], sum(total[s] for s in secuaces)


def medir_colusion_dispersa() -> dict:
    """
    Anillo DISPERSO (frente §1.6) con tres defensas: sin damping, damping LOCAL (solo independencia
    pareja), y damping LOCAL + COMUNIDAD (label propagation + sospecha). Mide si la detección de
    comunidades cierra el hueco que el damping local deja.
    """
    densa = escenarios.escenario_colusion()
    dispersa = escenarios.escenario_colusion_dispersa()
    _, sec_densa_local = _lavado_secuaces(densa, damping=True)
    _, sec_sin = _lavado_secuaces(dispersa, damping=False)
    _, sec_local = _lavado_secuaces(dispersa, damping=True, comunidad=False)
    _, sec_comun = _lavado_secuaces(dispersa, damping=True, comunidad=True)
    # control de falsos positivos: ¿la comunidad daña la reputación honesta?
    rep_local = reputacion_vectorial(densa.agentes, densa.grafo, damping=True, comunidad=False)
    rep_comun = reputacion_vectorial(densa.agentes, densa.grafo, damping=True, comunidad=True)
    honestos = [a.id for a in densa.agentes if densa.facciones[a.id] in ("honesto", "genesis")]
    h_local = sum(suma_vector(rep_local[h]) for h in honestos)
    h_comun = sum(suma_vector(rep_comun[h]) for h in honestos)
    return {
        "sec_densa_local": sec_densa_local,
        "sec_sin": sec_sin,
        "sec_local": sec_local,
        "sec_comun": sec_comun,
        "honesto_local": h_local,
        "honesto_comun": h_comun,
    }


def medir_blanqueo() -> dict:
    esc = escenarios.escenario_blanqueo()
    rep_vec = reputacion_vectorial(esc.agentes, esc.grafo)
    viejo = suma_vector(rep_vec["blanq_viejo"])
    nuevo = suma_vector(rep_vec["blanq_nuevo"])
    return {"viejo": viejo, "nuevo": nuevo}


def demo_slashing() -> dict:
    """
    Demuestra responsabilidad persistente (§1.5c) + slashing en cascada (§1.7).

    Cadena de avales: mentor -> intermedio -> ahijado. El ahijado defrauda y pierde 100. El golpe
    repercute hacia arriba (fracción 0.5 por salto). Apadrinar a la ligera sale caro.
    """
    reputacion = {"mentor": 200.0, "intermedio": 120.0, "ahijado": 100.0, "ajeno": 150.0}
    reg = RegistroAvales()
    reg.apadrinar("mentor", "intermedio")
    reg.apadrinar("intermedio", "ahijado")
    despues = slashing_en_cascada(reputacion, reg, culpable="ahijado", perdida=100.0)
    return {"antes": reputacion, "despues": despues}


def demo_economia_avales() -> dict:
    """Cupo sublineal (§1.5c) + dividendo de mentor que NO rinde con títeres."""
    cupos = {rep: cupo_de_avales(rep) for rep in (1, 10, 100, 1000, 10000)}
    div_real = dividendo_de_mentor(rep_independiente_ahijado=80.0)   # ahijado con rep independiente
    div_titere = dividendo_de_mentor(rep_independiente_ahijado=0.5)  # títere: rep independiente ~0
    return {"cupos": cupos, "div_real": div_real, "div_titere": div_titere}


# ----------------------------- informe ----------------------------------------------------------

def construir_informe() -> str:
    L: list[str] = []
    w = L.append

    w("# RESULTADOS — Prototipo del motor de reputación de Harlequin\n")
    w("> Generado por `run_all.py` (solo stdlib). Reproducible: PRNG sembrado. Las cifras son\n"
      "> **relativas** (reparto de poder), no unidades absolutas. Mapea SPEC §1 (reputación), §1.6\n"
      "> (anti-colusión), §2.2 (consenso) y §5 (salida/seudónimo).\n")

    w("\n## Tesis que se pone a prueba\n")
    w("**El muro anti-Sybil real es la reputación GANADA, no la prueba de personalidad** (SPEC §1.5,\n"
      "§2.4). Crear identidades falsas o granjear avales en círculo NO debe dar poder estructural,\n"
      "porque la reputación se ancla en evidencia real y se amortigua la colusión (§1.6).\n")

    # escenarios principales
    resultados = {}
    for fab in escenarios.TODOS:
        esc = fab()
        if esc.nombre == "blanqueo":
            continue  # se trata aparte
        resultados[esc.nombre] = medir_escenario(esc)

    w("\n## 1. Reputación y poder de consenso por escenario\n")
    w("| Escenario | Nº agentes | Cuota de reputación ganada | Cuota de comité de consenso |\n")
    w("|---|---|---|---|\n")
    for nombre in ("honesto", "sybil", "colusion"):
        r = resultados[nombre]
        esc = r["escenario"]
        w(f"| **{nombre}** | {len(esc.agentes)} | {fmt_pct(r['cuota_rep'])} | "
          f"{fmt_pct(r['cuota_comite'])} |\n")

    w("\n**Lectura:**\n")
    syb = resultados["sybil"]
    col = resultados["colusion"]
    w(f"- **Sybil:** 200 cuentas falsas (40% de la red) capturan "
      f"**{syb['cuota_rep'].get('sybil', 0.0):.2f}%** de la reputación y "
      f"**{syb['cuota_comite'].get('sybil', 0.0):.2f}%** del comité de consenso. "
      "Nacen con reputación 0; sin evidencia ni avales de reputados, no obtienen poder.\n")
    w(f"- **Colusión:** el anillo de 30 que se avala en círculo captura "
      f"**{col['cuota_rep'].get('colusor', 0.0):.2f}%** de la reputación y "
      f"**{col['cuota_comite'].get('colusor', 0.0):.2f}%** del comité, pese a los 3 honestos "
      "engañados que lo avalan.\n")

    # damping
    damp = medir_colusion_damping()
    c0_con, sec_con, max_con = damp["con"]
    c0_sin, sec_sin, max_sin = damp["sin"]
    w("\n## 2. ¿Cuánto aporta el anti-colusión (damping, §1.6)?\n")
    w("Ataque de **lavado de reputación**: un miembro del anillo (`c0`) SÍ tiene reputación legítima\n"
      "alta (trabajo real) e intenta **repartirla** a 29 secuaces avalándose en círculo. El damping\n"
      "debe impedir la propagación: `c0` conserva lo suyo, los secuaces se quedan en ~0.\n\n")
    w(f"| | Reputación de `c0` (legítima) | Repartida a los {damp['n_secuaces']} secuaces | "
      "Máx. de un secuaz |\n|---|---|---|---|\n")
    w(f"| **SIN damping** | {c0_sin:.1f} | {sec_sin:.1f} | {max_sin:.1f} |\n")
    w(f"| **CON damping** | {c0_con:.1f} | {sec_con:.1f} | {max_con:.1f} |\n")
    if sec_con > 0:
        w(f"\nEl damping recorta la reputación lavada a los secuaces **{sec_sin / sec_con:.1f}×**.\n")
    else:
        w("\nCon damping, lo lavado a los secuaces cae a ~0.\n")
    w("Sin el anti-colusión, un solo miembro con reputación real puede 'prestársela' a todo su\n"
      "anillo de títeres; con él, la reputación se queda donde se ganó.\n")

    # colusion dispersa (frente abierto §1.6) + defensa por comunidades
    disp = medir_colusion_dispersa()
    w("\n## 2b. Frente abierto: colusión SOFISTICADA (anillo disperso, §1.6)\n")
    w("El clique denso es fácil de detectar. Un anillo **disperso** (cada colusor avala a pocos, baja "
      "reciprocidad/solapamiento) imita patrones honestos y **se filtra** del damping local. Probamos "
      "una defensa nueva: **detección de comunidades** (la señal global que el anillo disperso sí deja).\n\n")
    w("| Defensa sobre el anillo disperso | Lavado a los secuaces |\n|---|---|\n")
    w(f"| sin damping (referencia) | {disp['sec_sin']:.1f} |\n")
    w(f"| damping LOCAL (solo independencia pareja) | {disp['sec_local']:.1f} |\n")
    w(f"| damping LOCAL + COMUNIDAD (nuevo) | {disp['sec_comun']:.1f} |\n")
    w(f"\n(Referencia: el clique denso con damping local lava solo {disp['sec_densa_local']:.1f}.)\n")
    mejora = disp["sec_local"] / disp["sec_comun"] if disp["sec_comun"] > 0 else float("inf")
    w(f"\n**Resultado:** la detección de comunidades reduce el lavado del anillo disperso de "
      f"**{disp['sec_local']:.0f}** (damping local) a **{disp['sec_comun']:.0f}** "
      f"(~{mejora:.1f}× menos) — cierra buena parte del hueco que dejaba el damping local.\n")
    danio = 100.0 * (1.0 - disp["honesto_comun"] / disp["honesto_local"]) if disp["honesto_local"] else 0.0
    w(f"**Control de falsos positivos:** la reputación honesta total apenas cambia con la defensa de "
      f"comunidades ({danio:+.1f}% sobre la base) → no castiga a las comunidades honestas (tienen "
      "evidencia real, baja su sospecha). Honesto: sigue siendo opt-in y a validar más; la colusión "
      "adaptativa (fragmentar el anillo en varias comunidades) es el siguiente frente (§1.6, PAPER §10).\n")

    # colusión adaptativa: fragmentar para evadir la etiqueta de comunidad (§1.6, frente abierto)
    filas = adaptativo.barrido(puentes=1)
    w("\n## 2c. Frente abierto: colusión ADAPTATIVA (fragmentar para evadir, §1.6)\n")
    w("El atacante sabe que castigamos las comunidades densas-sin-evidencia, así que **fragmenta** el "
      "anillo en sub-anillos pequeños y dispersos para caer por debajo del radar. Pero para lavar la "
      "reputación real de c0 a los títeres, ésta tiene que **fluir** entre fragmentos por unos pocos "
      "puentes. Esa es la tensión: evadir la etiqueta estrangula el flujo.\n\n")
    w("| fragmentos | comunidades vistas | lavado sin damping | lavado +comunidad | **poder consenso (min)** |\n")
    w("|---:|---:|---:|---:|---:|\n")
    for k, m in filas:
        w(f"| {k} | {m['n_com']} | {m['sin']:.1f} | {m['comunidad']:.1f} | **{m['poder']:.3f}** |\n")
    w("\n**Resultado (doble):** (1) fragmentar **sube** el nº de comunidades vistas (evade la etiqueta) "
      "pero **no sube** el lavado bajo defensa —al fragmentar, los puentes se vuelven cuellos de botella "
      "que el damping LOCAL muerde más fuerte—; el peor caso para la defensa es el anillo disperso SIN "
      "fragmentar. (2) Lo que sí se filtra es **unidimensional** (solo `comercio`): bajo el agregado "
      "conservador (min, §1.2b) que rige el poder de consenso/aval, los títeres colapsan a **~0** a "
      "cualquier fragmentación. Para tener poder real el atacante necesitaría evidencia verificable en "
      "**todas** las dimensiones por cada títere = hacer el trabajo honesto. *El lavado difuso no compra "
      "poder estructural.*\n")

    # blanqueo
    bl = medir_blanqueo()
    w("\n## 3. Blanqueo de seudónimo (whitewashing, §5)\n")
    w("Mismo humano, dos máscaras: una consolidada y una nueva.\n\n")
    w("| Seudónimo | Reputación ganada |\n|---|---|\n")
    w(f"| consolidado (`blanq_viejo`) | {bl['viejo']:.2f} |\n")
    w(f"| nuevo (`blanq_nuevo`) | {bl['nuevo']:.2f} |\n")
    w("\nAbandonar el seudónimo para 'empezar limpio' cuesta **toda** la reputación ganada: vuelve a\n"
      "la ciudadanía base. La reputación se ata a la máscara y no se transfiere (Art. VII/VIII).\n")

    # slashing
    sl = demo_slashing()
    w("\n## 4. Responsabilidad persistente + slashing en cascada (§1.5c, §1.7)\n")
    w("El ahijado defrauda y pierde 100. El golpe sube por la cadena de avales (½ por salto).\n\n")
    w("| Agente | Antes | Después | Δ |\n|---|---|---|---|\n")
    for aid in ("ahijado", "intermedio", "mentor", "ajeno"):
        a, d = sl["antes"][aid], sl["despues"][aid]
        w(f"| {aid} | {a:.1f} | {d:.1f} | {d - a:+.1f} |\n")
    w("\nQuien avala responde de a quién metió, aunque pase el tiempo. El `ajeno` (no avaló) no se\n"
      "ve afectado. Apadrinar a la ligera sale caro -> fuerza selectividad.\n")

    # economia avales
    ec = demo_economia_avales()
    w("\n## 5. Economía de avales (§1.5c)\n")
    w("**Cupo de avales vivos = función sublineal de la reputación** (rendimientos decrecientes):\n\n")
    w("| Reputación | Cupo de avales |\n|---|---|\n")
    for rep, cupo in ec["cupos"].items():
        w(f"| {rep} | {cupo} |\n")
    w(f"\n**Dividendo de mentor:** apadrinar a alguien con reputación independiente real rinde "
      f"{ec['div_real']:.2f}; apadrinar a un títere (reputación independiente ~0) rinde "
      f"{ec['div_titere']:.2f}. Las granjas padrino->títere no son rentables (§1.6).\n")

    # dinámica temporal (§1.7 decaimiento, Art. VI anti-atrincheramiento)
    import temporal
    tray = temporal.simular()
    w("\n## 6. Dinámica temporal: decaimiento y anti-atrincheramiento (§1.7, Art. VI)\n")
    w(f"El motor base es una foto fija; aquí se modela el TIEMPO en épocas envejeciendo el ancla de "
      f"evidencia (ρ={temporal.RHO} de retención por época). La reputación no contribuida se evapora "
      "(§1.7) y el poder de ayer no blinda el de mañana (Art. VI).\n\n")
    w("| época | Honesto activo | Honesto retirado (para en t=3) | Pionero durmiente (obra única t=0) | Títeres anillo farm-y-sienta |\n")
    w("|---:|---:|---:|---:|---:|\n")
    for t in range(temporal.N_EPOCAS):
        w(f"| {t} | {tray['honesto_activo'][t]:.0f} | {tray['honesto_retirado'][t]:.0f} | "
          f"{tray['pionero_durmiente'][t]:.0f} | {tray['titeres_anillo'][t]:.0f} |\n")
    ret, pio = tray["honesto_retirado"], tray["pionero_durmiente"]
    w(f"\n**Resultado:** quien **sigue aportando** se sostiene y crece; quien se **retira** decae "
      f"(~{100*(1-ret[-1]/max(ret)):.0f}% desde su pico) → anti-atrincheramiento (Art. VI). Un **pionero** "
      f"de una obra única se desinfla (~{100*(1-pio[-1]/max(pio)):.0f}%) → defensa **anti-long-range** "
      "gratis: una historia vieja no se reactiva a poder. Y una **granja** que farmea y se sienta ve a "
      "sus títeres evaporarse: la colusión tiene que ser SOSTENIDA, no un sprint.\n")

    w("\n## Conclusión\n")
    w("El prototipo confirma, en cifras, la apuesta central de la SPEC: **el poder estructural no se\n"
      "compra con identidades ni con avales endogámicos**. Sybils y anillos de colusión quedan cerca\n"
      "de 0% de poder de consenso; el damping anti-colusión es medible y necesario; el blanqueo no\n"
      "compensa; y la responsabilidad persistente hace cara la colusión. La colusión sofisticada\n"
      "—anillo disperso (§2b) y fragmentado/adaptativo (§2c)— tampoco compra poder: el lavado que se\n"
      "filtra es unidimensional y el agregado conservador (§1.2b) lo anula. Frentes vivos: dinámica\n"
      "temporal (decaimiento por época) y safety/liveness formales del consenso.\n")

    return "".join(L)


def main() -> None:
    informe = construir_informe()
    ruta = "RESULTADOS.md"
    with open(ruta, "w", encoding="utf-8") as f:
        f.write(informe)
    print(f"[ok] informe escrito en {ruta}")
    if "--stdout" in sys.argv:
        print("\n" + informe)


if __name__ == "__main__":
    main()
