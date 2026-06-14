#!/usr/bin/env python3
"""
Consenso bajo PARTICIÓN de red — endurece el modelo (limitación honesta: antes era red completa).

Antes de comprometer el stack de la cadena, hay que saber cómo se comporta WTC cuando la red se
**parte** (un grupo de nodos queda aislado un tiempo y luego sana). Es el escenario clásico que
distingue safety de liveness y donde viven los ataques de partición.

Modelo: los honestos se reparten en un grupo GRANDE (A) y uno PEQUEÑO (B). El adversario concentra
su reputación en B. Durante `D` rondas la red está partida (cada grupo solo se muestrea a sí mismo);
después sana (red completa). Se mide sobre muchas ejecuciones:
  - **fork**: A y B deciden colores DISTINTOS (fallo de SAFETY).
  - **captura en B**: algún honesto de B adopta el valor falso.
  - **atasco**: alguien queda indeciso al final (coste de LIVENESS).

Hipótesis honesta a comprobar: una partición LARGA puede elevar la cuota LOCAL del adversario en el
grupo pequeño por encima del umbral y capturarlo / finalizarlo → al sanar, fork. Es un ataque real
(partición + adversario concentrado). Sirve para MEDIR la frontera y motivar la mitigación (no
finalizar bajo sospecha de partición / requerir ver una fracción mínima de la red).

Ejecutar desde prototipos/consenso/:  python3 particion.py
"""

from __future__ import annotations

import os
import random
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from wtc_sim.consenso import ParamsConsenso, run_once

P = ParamsConsenso(k=20, alpha=14, beta=12, max_rondas=120)
TRIALS = 60
SEMILLA = 1984


def poblacion_particionada(f_adv: float, n_A: int = 60, n_B: int = 20, n_adv: int = 6):
    """
    Honestos en grupo A (grande) y B (pequeño). Adversario (fracción `f_adv` de la reputación TOTAL)
    TODO en B. Devuelve (reputacion, adversarios, grupo) con grupo: id -> 0 (A) | 1 (B).
    """
    reputacion: dict[str, float] = {}
    grupo: dict[str, int] = {}
    for i in range(n_A):
        reputacion[f"a{i}"] = 1.0; grupo[f"a{i}"] = 0
    for i in range(n_B):
        reputacion[f"b{i}"] = 1.0; grupo[f"b{i}"] = 1
    total_h = float(n_A + n_B)
    adversarios: set[str] = set()
    if f_adv > 0:
        adv_total = f_adv * total_h / (1.0 - f_adv)
        for i in range(n_adv):
            aid = f"x{i}"
            reputacion[aid] = adv_total / n_adv
            grupo[aid] = 1                # el adversario vive en el grupo pequeño B
            adversarios.add(aid)
    return reputacion, adversarios, grupo


def medir(f_adv, D, quorum_red=0.0, trials=TRIALS):
    rng = random.Random(SEMILLA)
    rep, adv, grupo = poblacion_particionada(f_adv)
    fork = captura = atasco = seguro = 0
    for _ in range(trials):
        r = run_once(rep, adv, P, rng, ponderado=True, grupo=grupo,
                     rondas_particion=D, quorum_red=quorum_red)
        fork += r["bifurcacion"]; captura += r["captura"]
        atasco += 1 if r["indecisos"] > 0 else 0
        seguro += r["seguro"]
    n = float(trials)
    return {"fork": 100*fork/n, "captura": 100*captura/n, "atasco": 100*atasco/n, "seguro": 100*seguro/n}


def main():
    print("# Consenso bajo partición de red\n")
    print("Honestos: A=60 (grande), B=20 (pequeño). Adversario (15% global) concentrado en B. "
          "Partición de D rondas, luego sana.\n")
    print("## Sin mitigación: el ataque de partición\n")
    print(f"{'D (rondas)':>10} | {'fork':>6} | {'captura':>8} | {'atasco':>7} | {'seguro':>7}")
    print("-" * 50)
    for D in (0, 20, 50, 90):
        m = medir(0.15, D)
        print(f"{D:>10} | {m['fork']:>5.0f}% | {m['captura']:>7.0f}% | {m['atasco']:>6.0f}% | {m['seguro']:>6.0f}%")
    print("\nUna partición larga concentra la cuota LOCAL del adversario en B por encima del umbral: "
          "B finaliza el valor falso → al sanar, FORK. Un 15% global inofensivo se vuelve 100% fork.\n")

    print("## Con mitigación quórum-de-red (un nodo no finaliza si ve < 60% de la reputación)\n")
    print(f"{'D (rondas)':>10} | {'fork':>6} | {'captura':>8} | {'atasco':>7} | {'seguro':>7}")
    print("-" * 50)
    for D in (0, 20, 50, 90):
        m = medir(0.15, D, quorum_red=0.6)
        print(f"{D:>10} | {m['fork']:>5.0f}% | {m['captura']:>7.0f}% | {m['atasco']:>6.0f}% | {m['seguro']:>6.0f}%")
    print("\nB (25% de la reputación) nunca alcanza el quórum durante la partición → NO finaliza → se "
          "atasca (coste de liveness) en vez de forkear, y se recupera al sanar. Safety preservada: "
          "0% fork. La finalidad se condiciona a ver suficiente red (anti-partición).")


if __name__ == "__main__":
    main()
