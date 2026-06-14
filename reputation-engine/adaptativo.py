#!/usr/bin/env python3
"""
Barrido del ataque de colusión ADAPTATIVA (frente abierto §1.6).

Mide la tensión central del atacante sofisticado: para evadir la detección de comunidades
fragmenta el anillo en sub-anillos pequeños y dispersos; pero para LAVAR la reputación real de su
único nodo con evidencia (c0) a los títeres, la reputación tiene que FLUIR entre fragmentos por
unos pocos puentes. Hipótesis: no puede tener las dos cosas —evadir Y lavar mucho—.

Se barre `n_fragmentos` (1 = anillo disperso clásico ... hasta muy fragmentado) y, para cada punto,
se mide la reputación TOTAL que capturan los 29 títeres (c1..c29, evidencia 0), con tres regímenes:
  - sin damping            (control: cuánto lavaría sin ninguna defensa)
  - damping local          (independencia de aristas, §1.6 primera línea)
  - damping + comunidad     (defensa global por comunidad, §1.6 tercera línea)

También reporta cuántas comunidades "ve" la detección de etiquetas: si crece con la fragmentación,
el atacante SÍ evade la etiqueta —pero hay que mirar si el lavado sube o no—.

Ejecutar desde prototipos/reputacion/:  python3 adaptativo.py
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from harlequin_rep.reputacion import agregado_conservador, reputacion_vectorial
import escenarios


def _suma(vec: dict[str, float]) -> float:
    return sum(vec.values())


def _titeres(esc) -> list[str]:
    return [c for c, f in esc.facciones.items() if f == "colusor" and c != "c0"]


def _honestos(esc) -> list[str]:
    return [a.id for a in esc.agentes if esc.facciones[a.id] in ("honesto", "genesis")]


def _n_comunidades_anillo(esc) -> int:
    """Cuántas comunidades distintas etiqueta la detección DENTRO del anillo de colusión."""
    nodos = [a.id for a in esc.agentes]
    etiqueta = esc.grafo.comunidades("comercio", nodos)
    anillo = [c for c, f in esc.facciones.items() if f == "colusor"]
    return len(set(etiqueta[c] for c in anillo))


def medir(esc):
    titeres, honestos = _titeres(esc), _honestos(esc)

    def lavado(damping, comunidad):
        rep = reputacion_vectorial(esc.agentes, esc.grafo, damping=damping, comunidad=comunidad)
        # lavado bruto (suma de todas las dimensiones) de los títeres
        bruto = sum(_suma(rep[t]) for t in titeres)
        # PODER DE CONSENSO real: agregado CONSERVADOR (min) del vector por títere (§1.2b). Como solo
        # lavan en 'comercio' y tienen 0 en las otras 3 dimensiones, el min debe colapsar a ~0.
        poder = sum(agregado_conservador(rep[t], "min") for t in titeres)
        hon = sum(_suma(rep[h]) for h in honestos)
        return bruto, poder, hon

    sin, _, _ = lavado(False, False)
    loc, _, hon_loc = lavado(True, False)
    com, poder_com, hon_com = lavado(True, True)
    return {
        "sin": sin, "local": loc, "comunidad": com,
        "poder": poder_com, "hon_comunidad": hon_com,
        "n_com": _n_comunidades_anillo(esc),
    }


def barrido(n_anillo=30, fragmentos=(1, 2, 3, 5, 6, 10, 15), puentes=1, semilla=7):
    filas = []
    for k in fragmentos:
        esc = escenarios.escenario_colusion_adaptativa(
            semilla=semilla, n_anillo=n_anillo, n_fragmentos=k, puentes=puentes
        )
        m = medir(esc)
        filas.append((k, m))
    return filas


def formatear(filas, puentes) -> str:
    out = []
    out.append(f"### Barrido de fragmentación (anillo=30, {puentes} puente(s)/par de fragmentos)\n")
    out.append("Reputación TOTAL capturada por los 29 títeres (evidencia 0). Más bajo = mejor defensa.\n")
    out.append("| fragmentos | comunidades vistas | lavado sin damping | lavado local | lavado +comunidad | **poder consenso (min)** |")
    out.append("|---:|---:|---:|---:|---:|---:|")
    for k, m in filas:
        out.append(
            f"| {k} | {m['n_com']} | {m['sin']:.1f} | {m['local']:.1f} | "
            f"{m['comunidad']:.1f} | **{m['poder']:.3f}** |"
        )
    out.append("")
    return "\n".join(out)


def main():
    print("# Colusión adaptativa — barrido de fragmentación\n")
    for puentes in (1, 2):
        filas = barrido(puentes=puentes)
        print(formatear(filas, puentes))


if __name__ == "__main__":
    main()
