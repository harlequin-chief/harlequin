#!/usr/bin/env python3
"""
Envejecimiento de ARISTAS (avales) por época (§1.7, Art. VI) — cierra la limitación de `temporal.py`.

`temporal.py` envejece el ANCLA de evidencia pero deja los avales (aristas del grafo) estáticos. Aquí
se envejecen también los avales: un aval pesa `ρ_arista^(edad)` y se evapora si no se RENUEVA. Modela
que la confianza es perecedera —un aval viejo, de alguien que ya no trata contigo, dice menos que uno
fresco— y que por tanto la reputación que descansa en avales rancios decae aunque la evidencia siga.

Experimento CONTROLADO (aísla el efecto del edge-aging):
  - dos honestos con la MISMA evidencia constante y los MISMOS avales iniciales;
  - `honesto_renovador` recibe un aval fresco cada época (contrapartes que siguen tratando con él);
  - `honesto_durmiente` recibió sus avales al inicio y NUNCA los renueva.
Cualquier diferencia de reputación entre ambos es, por construcción, SOLO del envejecimiento de
aristas. Además, un anillo de colusión que "farmea" sus avales mutuos en t=0 y se sienta los ve
decaer: la colusión, también por el grafo, tiene que ser SOSTENIDA.

Implementación NO invasiva: no se toca el core. Cada época se reconstruye un grafo nuevo desde un
LOG de avales con época de emisión, aplicando el peso envejecido. El motor se usa tal cual.

Ejecutar desde prototipos/reputacion/:  python3 aristas.py
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from harlequin_rep.graph import GrafoConfianza
from harlequin_rep.model import DIMENSIONES, Agente, TipoAgente
from harlequin_rep.reputacion import reputacion_vectorial

DIM = "comercio"
N_EPOCAS = 10
RHO_ARISTA = 0.7   # retención de un aval por época; un aval sin renovar pierde 30% cada época
SEMILLA = 7


def _construir():
    """Agentes con evidencia CONSTANTE + log de avales con época de emisión (para envejecer)."""
    import random
    rng = random.Random(SEMILLA)

    agentes: list[Agente] = []
    facciones: dict[str, str] = {}
    # log de avales: (origen, destino, dim, epoca_emision). Se renueva añadiendo entradas nuevas.
    log: list[tuple[str, str, str, int]] = []

    genesis = [f"g{i}" for i in range(4)]
    for gid in genesis:
        agentes.append(Agente(id=gid, tipo=TipoAgente.GENESIS, evidencia={d: 2.0 for d in DIMENSIONES}))
        facciones[gid] = "genesis"

    fondo = [f"p{i}" for i in range(12)]
    for pid in fondo:
        agentes.append(Agente(id=pid, tipo=TipoAgente.HONESTO, evidencia={DIM: rng.uniform(2.0, 4.0)}))
        facciones[pid] = "fondo"
        for av in rng.sample(genesis, 2):
            log.append((av, pid, DIM, 0))

    # dos honestos con la MISMA evidencia y los MISMOS 4 avales iniciales (en t=0)
    for sid in ("honesto_renovador", "honesto_durmiente"):
        agentes.append(Agente(id=sid, tipo=TipoAgente.HONESTO, evidencia={DIM: 3.0}))
        facciones[sid] = "seguido"
        for av in rng.sample(fondo, 4):
            log.append((av, sid, DIM, 0))

    # anillo de colusión: farmea sus avales mutuos en t=0 y se sienta (no renueva). c0 con evidencia.
    anillo = [f"c{i}" for i in range(10)]
    for idx, cid in enumerate(anillo):
        ev = {DIM: 8.0} if idx == 0 else {}
        agentes.append(Agente(id=cid, tipo=TipoAgente.COLUSOR, evidencia=ev, cluster="farm"))
        facciones[cid] = "colusor"
    for a in anillo:
        for b in anillo:
            if a != b:
                log.append((a, b, DIM, 0))

    return agentes, facciones, log, fondo


def simular(rho_arista: float = RHO_ARISTA):
    agentes, facciones, log, fondo = _construir()
    anillo = [a.id for a in agentes if facciones[a.id] == "colusor"]

    # renovaciones: el renovador recibe un aval fresco cada época de un avalador DISTINTO
    # (contrapartes nuevas que siguen tratando con él); el durmiente NUNCA renueva.
    renovaciones: list[tuple[str, str, str, int]] = []
    for t in range(1, N_EPOCAS):
        av = fondo[t % len(fondo)]   # round-robin determinista -> curva limpia
        renovaciones.append((av, "honesto_renovador", DIM, t))

    def grafo(t):
        g = GrafoConfianza()
        for (o, d, dim, ep) in (list(log) + renovaciones):
            if ep <= t:
                g.atestar(o, d, dim, rho_arista ** (t - ep))
        return g

    tray = {"renovador": [], "durmiente": [], "titeres_anillo": []}
    for t in range(N_EPOCAS):
        rep = reputacion_vectorial(agentes, grafo(t), damping=True)
        tray["renovador"].append(sum(rep["honesto_renovador"].values()))
        tray["durmiente"].append(sum(rep["honesto_durmiente"].values()))
        tray["titeres_anillo"].append(sum(sum(rep[c].values()) for c in anillo if c != "c0"))
    return tray


def formatear(tray, control) -> str:
    out = ["# Envejecimiento de aristas (avales) por época (§1.7, Art. VI)\n"]
    out.append(f"ρ_arista={RHO_ARISTA} (un aval sin renovar pierde {int((1-RHO_ARISTA)*100)}% por época). "
               f"{N_EPOCAS} épocas. Reputación ganada.\n")
    out.append("Experimento CONTROLADO: dos honestos con **idéntica evidencia constante** y mismos avales "
               "iniciales; el `renovador` recibe un aval fresco cada época, el `durmiente` no. La columna "
               "`durmiente (sin aging, ρ=1)` es el control que aísla lo que aporta el envejecimiento.\n")
    out.append("| época | renovador | durmiente | durmiente (sin aging, ρ=1) | títeres anillo |")
    out.append("|---:|---:|---:|---:|---:|")
    for t in range(N_EPOCAS):
        out.append(f"| {t} | {tray['renovador'][t]:.1f} | {tray['durmiente'][t]:.1f} | "
                   f"{control['durmiente'][t]:.1f} | {tray['titeres_anillo'][t]:.1f} |")
    out.append("")
    r, d = tray["renovador"], tray["durmiente"]
    dc = control["durmiente"]
    brecha = 100 * (r[-1] / max(d[-1], 1e-9) - 1)
    extra_aging = 100 * (1 - d[-1] / max(dc[-1], 1e-9))
    out.append("**Lecturas (honestas):**")
    out.append(f"- **Prima de frescura:** misma evidencia, el **renovador** acaba ~{brecha:.0f}% por encima "
               "del **durmiente**. Mantener la confianza VIVA pesa; descansar en avales viejos, no.")
    out.append(f"- **Lo que aporta el aging:** ya sin envejecer (ρ=1) el durmiente cae (las aristas frescas "
               f"ajenas lo diluten por la normalización fila-estocástica); el envejecimiento lo recorta "
               f"un ~{extra_aging:.0f}% ADICIONAL ({dc[-1]:.0f}→{d[-1]:.0f}). El anti-atrincheramiento "
               "(Art. VI) alcanza también al GRAFO de confianza, no solo al ancla de evidencia.")
    out.append("- **Matiz honesto:** el decaimiento uniforme de TODAS las aristas de un nodo se cancela en "
               "parte al normalizar la fila (mismo motivo que el bug del damping ya corregido); el efecto "
               "real es RELATIVO —no renovar mientras otros sí—, que es justo lo que se quiere premiar.")
    out.append("- El **anillo farm-y-sienta** apenas se mueve porque YA está cerca de 0 por el damping "
               "(no le queda reputación que envejecer): aquí el aging es presión menor, la primera línea "
               "es el anclaje en evidencia + independencia.")
    out.append("")
    return "\n".join(out)


def main():
    print(formatear(simular(), control=simular(rho_arista=1.0)))


if __name__ == "__main__":
    main()
