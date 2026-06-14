#!/usr/bin/env python3
"""
Dinámica TEMPORAL del motor de reputación (§1.7 decaimiento, Art. VI anti-atrincheramiento).

El motor base es estático: calcula la reputación de una foto fija. Pero la SPEC §1.7 dice que la
reputación **no contribuida se evapora**, y el manifiesto (Art. VI) prohíbe el atrincheramiento: el
poder de ayer no puede blindar el de mañana. Aquí se modela el TIEMPO en épocas.

Implementación del decaimiento (la más principista): se envejece el ANCLA, no un marcador aparte.
La reputación se ancla en evidencia (pre-trust); la evidencia ANTIGUA pierde peso de forma
exponencial. En la época t, la evidencia efectiva de un agente es

    ancla_t[dim] = Σ_{s≤t}  evidencia_bruta_s[dim] · ρ^(t−s)         (ρ = retención por época)

Así, recomputar el EigenTrust época a época da una reputación que:
  - sube y se sostiene si SIGUES aportando (honesto activo),
  - sube y luego DECAE si dejas de aportar (honesto retirado) → anti-atrincheramiento (Art. VI),
  - un pionero con una gran obra única que luego se duerme NO conserva el poder para siempre
    → defensa anti-long-range GRATIS (una historia vieja no puede reactivarse a poder),
  - una granja de colusión que farmea y se sienta se desinfla → la colusión tiene que ser sostenida.

Limitación honesta: los avales (grafo) se modelan estáticos; solo envejece la evidencia. Una
iteración futura puede envejecer también las aristas y graduar ahijados por época.

Ejecutar desde prototipos/reputacion/:  python3 temporal.py
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import random

from harlequin_rep.graph import GrafoConfianza
from harlequin_rep.model import DIMENSIONES, Agente, TipoAgente
from harlequin_rep.reputacion import reputacion_vectorial

RHO = 0.7          # retención de evidencia por época (decaimiento §1.7); 1−ρ se evapora
N_EPOCAS = 10
SEMILLA = 7

# perfiles seguidos (id -> (etiqueta legible, función evidencia_bruta(epoca) -> {dim: monto}))
DIM_A, DIM_B = "comercio", "contribucion_tecnica"


def _stream_activo(t: int) -> dict[str, float]:
    """Aporta trabajo real cada época en 2 dimensiones."""
    return {DIM_A: 4.0, DIM_B: 3.0}


def _stream_retirado(t: int) -> dict[str, float]:
    """Aporta fuerte las épocas 0-2, luego se retira (deja de contribuir)."""
    return {DIM_A: 5.0, DIM_B: 4.0} if t <= 2 else {}


def _stream_pionero(t: int) -> dict[str, float]:
    """Una sola gran obra en la época 0 (pionero) y luego silencio (caso long-range)."""
    return {DIM_A: 30.0, DIM_B: 25.0} if t == 0 else {}


def _stream_farm(t: int) -> dict[str, float]:
    """Granja de colusión: el ancla (c0) farmea fuerte épocas 0-1, luego se sienta a recircular."""
    return {DIM_A: 20.0} if t <= 1 else {}


PERFILES = {
    "honesto_activo":   ("Honesto activo (aporta siempre)", _stream_activo),
    "honesto_retirado": ("Honesto retirado (aporta y para en t=3)", _stream_retirado),
    "pionero_durmiente": ("Pionero (gran obra única en t=0, luego duerme)", _stream_pionero),
}


def _construir(rng: random.Random):
    """Red base honesta + los agentes-perfil seguidos + un anillo de colusión que farmea-y-se-sienta."""
    agentes: list[Agente] = []
    facciones: dict[str, str] = {}
    grafo = GrafoConfianza()

    # génesis (semilla diluyente) + relleno honesto de fondo
    genesis = [f"g{i}" for i in range(5)]
    for gid in genesis:
        agentes.append(Agente(id=gid, tipo=TipoAgente.GENESIS, evidencia={d: 2.0 for d in DIMENSIONES}))
        facciones[gid] = "genesis"
    fondo = [f"h{i}" for i in range(20)]
    for hid in fondo:
        agentes.append(Agente(id=hid, tipo=TipoAgente.HONESTO, evidencia={}))
        facciones[hid] = "fondo"

    # agentes-perfil seguidos (su evidencia se inyecta por época; aquí van sin evidencia inicial)
    seguidos = list(PERFILES)
    for sid in seguidos:
        agentes.append(Agente(id=sid, tipo=TipoAgente.HONESTO, evidencia={}))
        facciones[sid] = "seguido"

    # cada seguido recibe avales independientes del fondo (confianza real, baja endogamia)
    establecidos = genesis + fondo
    for sid in seguidos:
        for av in rng.sample(establecidos, 4):
            for d in (DIM_A, DIM_B):
                grafo.atestar(av, sid, d, 1.0)
    # el fondo también recibe algo de aval para no estar a cero (textura de red)
    for hid in fondo:
        for av in rng.sample([e for e in establecidos if e != hid], 2):
            grafo.atestar(av, hid, DIM_A, 1.0)

    # anillo de colusión farm-y-sienta: c0 con evidencia por época (stream_farm), c1..c9 títeres
    anillo = [f"c{i}" for i in range(10)]
    for cid in anillo:
        agentes.append(Agente(id=cid, tipo=TipoAgente.COLUSOR, evidencia={}, cluster="farm"))
        facciones[cid] = "colusor"
    for a in anillo:               # clique reciprocal (granja)
        for b in anillo:
            if a != b:
                grafo.atestar(a, b, DIM_A, 1.0)

    return agentes, grafo, facciones, seguidos, anillo


def simular():
    rng = random.Random(SEMILLA)
    agentes, grafo, facciones, seguidos, anillo = _construir(rng)

    # historia de evidencia bruta por época para los agentes con stream
    streams = {sid: PERFILES[sid][1] for sid in seguidos}
    streams["c0"] = _stream_farm   # el ancla del anillo

    base = {a.id: dict(a.evidencia) for a in agentes}  # evidencia estática (génesis)

    trayectoria: dict[str, list[float]] = {sid: [] for sid in seguidos}
    trayectoria["titeres_anillo"] = []   # suma de c1..c9

    for t in range(N_EPOCAS):
        # ancla envejecida: Σ_{s≤t} bruta_s · ρ^(t−s)
        for a in agentes:
            aged: dict[str, float] = dict(base[a.id])  # evidencia estática no envejece (génesis)
            if a.id in streams:
                acc: dict[str, float] = {}
                for s in range(t + 1):
                    bruta = streams[a.id](s)
                    peso = RHO ** (t - s)
                    for d, v in bruta.items():
                        acc[d] = acc.get(d, 0.0) + v * peso
                for d, v in acc.items():
                    aged[d] = aged.get(d, 0.0) + v
            a.evidencia = aged

        rep = reputacion_vectorial(agentes, grafo, damping=True)
        for sid in seguidos:
            trayectoria[sid].append(sum(rep[sid].values()))
        trayectoria["titeres_anillo"].append(sum(sum(rep[c].values()) for c in anillo if c != "c0"))

    return trayectoria


def formatear(tray) -> str:
    out = ["# Dinámica temporal del motor (§1.7 decaimiento, Art. VI anti-atrincheramiento)\n"]
    out.append(f"ρ = {RHO} (retención de evidencia por época), {N_EPOCAS} épocas. Reputación ganada "
               "(suma del vector) por época.\n")
    etiquetas = {
        "honesto_activo": "Honesto activo",
        "honesto_retirado": "Honesto retirado (para en t=3)",
        "pionero_durmiente": "Pionero durmiente (obra única t=0)",
        "titeres_anillo": "Títeres del anillo farm-y-sienta",
    }
    cabecera = "| época | " + " | ".join(etiquetas[k] for k in etiquetas) + " |"
    out.append(cabecera)
    out.append("|---:|" + "---:|" * len(etiquetas))
    for t in range(N_EPOCAS):
        celdas = " | ".join(f"{tray[k][t]:.1f}" for k in etiquetas)
        out.append(f"| {t} | {celdas} |")
    out.append("")
    # lecturas automáticas
    activo = tray["honesto_activo"]
    ret = tray["honesto_retirado"]
    pio = tray["pionero_durmiente"]
    pico_ret = max(ret)
    caida_ret = 100.0 * (1.0 - ret[-1] / pico_ret) if pico_ret else 0.0
    caida_pio = 100.0 * (1.0 - pio[-1] / max(pio)) if max(pio) else 0.0
    out.append(f"**Lecturas:**")
    out.append(f"- Honesto **activo**: se sostiene (t0={activo[0]:.0f} → t{N_EPOCAS-1}={activo[-1]:.0f}). "
               "Aportar mantiene el poder.")
    out.append(f"- Honesto **retirado** (deja de aportar en t=3): pico {pico_ret:.0f} → "
               f"{ret[-1]:.0f} ({caida_ret:.0f}% menos). **Anti-atrincheramiento (Art. VI)**: el poder "
               "de ayer no se conserva sin obra nueva.")
    out.append(f"- **Pionero durmiente** (una gran obra en t=0, luego nada): {max(pio):.0f} → "
               f"{pio[-1]:.0f} ({caida_pio:.0f}% menos). **Anti-long-range gratis**: una historia vieja "
               "no puede reactivarse a poder presente.")
    out.append(f"- **Granja de colusión** que farmea (t≤1) y se sienta: los títeres se desinflan "
               f"({tray['titeres_anillo'][1]:.0f} en t=1 → {tray['titeres_anillo'][-1]:.0f} al final). "
               "La colusión tiene que ser SOSTENIDA en el tiempo, no un sprint.")
    out.append("")
    return "\n".join(out)


def main():
    tray = simular()
    print(formatear(tray))


if __name__ == "__main__":
    main()
