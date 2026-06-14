#!/usr/bin/env python3
"""
Graduación de ahijados (§1.5c): el apadrinamiento es ANDAMIAJE, no una correa permanente.

Un recién llegado (ahijado A) entra apadrinado por un mentor M: al principio su reputación se apoya
en el aval de M (el único que tiene). A medida que A hace obra real y recibe avales INDEPENDIENTES
(de contrapartes con las que trató), su reputación pasa a sostenerse SOLA. Cuando A ya se sostiene sin
el aval de M, **gradúa**: el vínculo deja de estar `vivo` y **libera el cupo de avales de M** (que
puede apadrinar a otro). La RESPONSABILIDAD persiste (el slashing en cascada sigue alcanzando a M).

Se mide en cada época, recomputando el motor:
  - rep_total(A)         : reputación de A CON el aval de M presente.
  - rep_independiente(A) : reputación de A QUITANDO el aval de M (lo que A sostiene por sí mismo).
  - graduación cuando rep_independiente(A) ≥ umbral · rep_total(A): A ya no depende del andamiaje.
  - cupo libre de M = cupo_de_avales(rep(M)) − avales_vivos(M): sube al graduar A.

Ejecutar desde prototipos/reputacion/:  python3 graduacion.py
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import random

from harlequin_rep.graph import GrafoConfianza
from harlequin_rep.model import DIMENSIONES, Agente, TipoAgente
from harlequin_rep.reputacion import reputacion_dimension
from harlequin_rep.vouch import RegistroAvales, cupo_de_avales

DIM = "comercio"
N_EPOCAS = 8
UMBRAL_GRADUACION = 0.6   # A gradúa cuando ≥60% de su reputación se sostiene SIN el aval del mentor
RHO = 0.8


def _construir():
    """Base honesta + mentor M (con obra real) + ahijado A (entra a cero) + contrapartes."""
    rng = random.Random(7)
    agentes: list[Agente] = []
    grafo = GrafoConfianza()

    genesis = [f"g{i}" for i in range(4)]
    for gid in genesis:
        agentes.append(Agente(id=gid, tipo=TipoAgente.GENESIS, evidencia={d: 2.0 for d in DIMENSIONES}))

    # contrapartes independientes (harán tratos con A y lo avalarán según pase el tiempo)
    contrapartes = [f"p{i}" for i in range(8)]
    for pid in contrapartes:
        agentes.append(Agente(id=pid, tipo=TipoAgente.HONESTO, evidencia={DIM: rng.uniform(3.0, 6.0)}))
        for av in rng.sample(genesis, 2):
            grafo.atestar(av, pid, DIM, 1.0)

    # mentor: obra real consolidada
    agentes.append(Agente(id="M", tipo=TipoAgente.HONESTO, evidencia={DIM: 12.0}))
    for av in rng.sample(genesis + contrapartes, 4):
        grafo.atestar(av, "M", DIM, 1.0)

    # ahijado: entra a cero, solo con el aval del mentor (puerta 1 = ciudadanía base)
    agentes.append(Agente(id="A", tipo=TipoAgente.HONESTO, evidencia={}))

    reg = RegistroAvales()
    reg.apadrinar("M", "A")
    grafo.atestar("M", "A", DIM, 1.0)   # el aval del mentor (andamiaje)

    return agentes, grafo, reg, contrapartes


def _stream_ahijado(t: int) -> float:
    """Evidencia (obra real) que A acumula por época: empieza pequeña y crece (se asienta)."""
    return 2.0 * t   # 0, 2, 4, 6, ... obra creciente


def simular():
    agentes, grafo, reg, contrapartes = _construir()
    base_ev = {a.id: dict(a.evidencia) for a in agentes}
    A = next(a for a in agentes if a.id == "A")

    tray = {"total": [], "independiente": [], "cupo_libre_M": [], "graduado": []}
    graduado = False

    for t in range(N_EPOCAS):
        # A acumula evidencia envejecida + recibe avales independientes nuevos según crece
        acc = 0.0
        for s in range(t + 1):
            acc += _stream_ahijado(s) * (RHO ** (t - s))
        A.evidencia = {DIM: acc} if acc > 0 else {}
        # cada época, una contraparte nueva que trató con A lo avala (aval independiente, no del mentor)
        if 0 < t <= len(contrapartes):
            grafo.atestar(contrapartes[t - 1], "A", DIM, 1.0)

        # rep CON aval del mentor vs SIN él (lo que A sostiene solo)
        rep_total = reputacion_dimension(agentes, grafo, DIM)
        # quitar temporalmente el aval del mentor para medir la reputación independiente de A
        peso_M = grafo.salientes("M", DIM).get("A", 0.0)
        grafo._aristas[DIM]["M"].pop("A", None)
        rep_indep = reputacion_dimension(agentes, grafo, DIM)
        if peso_M:                      # restaurar el aval
            grafo._aristas[DIM]["M"]["A"] = peso_M

        rt, ri = rep_total["A"], rep_indep["A"]
        # graduación: A se sostiene sin el andamiaje
        if not graduado and rt > 0 and ri >= UMBRAL_GRADUACION * rt:
            reg.graduar("M", "A")
            graduado = True

        cupo_libre = cupo_de_avales(rep_total["M"]) - reg.avales_vivos("M")

        tray["total"].append(rt)
        tray["independiente"].append(ri)
        tray["cupo_libre_M"].append(cupo_libre)
        tray["graduado"].append(graduado)

    return tray


def formatear(tray) -> str:
    out = ["# Graduación de ahijados (§1.5c): el apadrinamiento es andamiaje, no correa\n"]
    out.append(f"ρ={RHO}, umbral de graduación = {int(UMBRAL_GRADUACION*100)}% de reputación independiente. "
               f"{N_EPOCAS} épocas.\n")
    out.append("| época | rep total A | rep independiente A | % independiente | cupo libre M | ¿graduado? |")
    out.append("|---:|---:|---:|---:|---:|:--:|")
    for t in range(N_EPOCAS):
        rt = tray["total"][t]; ri = tray["independiente"][t]
        pct = 100 * ri / rt if rt else 0.0
        grad = "✓" if tray["graduado"][t] else "—"
        out.append(f"| {t} | {rt:.1f} | {ri:.1f} | {pct:.0f}% | {tray['cupo_libre_M'][t]} | {grad} |")
    out.append("")
    ep_grad = next((t for t in range(N_EPOCAS) if tray["graduado"][t]), None)
    out.append("**Lecturas:**")
    out.append("- Al inicio A depende del aval del mentor: su reputación **independiente** es una fracción "
               "pequeña de la total (el andamiaje sostiene el resto).")
    if ep_grad is not None:
        out.append(f"- En la **época {ep_grad}**, A ya se sostiene solo (≥{int(UMBRAL_GRADUACION*100)}% "
                   "independiente) → **gradúa**: el aval del mentor se libera y deja de ocupar su cupo "
                   f"(cupo libre de M sube de {tray['cupo_libre_M'][ep_grad-1]} a {tray['cupo_libre_M'][ep_grad]}).")
    out.append("- Tras graduar, A mantiene su reputación por obra propia; M recupera capacidad de "
               "apadrinar a otro. La **responsabilidad persiste** (el vínculo sigue registrado: el "
               "slashing en cascada seguiría alcanzando a M si A defraudara).")
    out.append("- Moraleja de incentivos: apadrinar bien es invertir en que el ahijado **crezca y se "
               "independice**, no en atarlo. El andamiaje se diseña para diluirse (coherente con la "
               "semilla génesis y el anti-atrincheramiento, Art. VI).")
    out.append("")
    return "\n".join(out)


def main():
    print(formatear(simular()))


if __name__ == "__main__":
    main()
