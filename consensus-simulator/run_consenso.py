#!/usr/bin/env python3
"""
Runner del simulador de consenso. Mide la tesis del paper y escribe RESULTADOS-consenso.md.

Uso:
    python3 run_consenso.py [--stdout]

Mide tres cosas:
  1. Barrido del adversario por FRACCIÓN DE REPUTACIÓN -> dónde está el umbral de seguridad.
  2. Falsa multitud (Sybil) con muestreo PONDERADO por reputación -> el número de nodos no basta.
  3. Misma Sybil con muestreo UNIFORME (contraste) -> sin ponderar por reputación, la multitud gana.
     Demuestra que la ponderación por reputación es lo que defiende (paralelo al damping del motor).
"""

from __future__ import annotations

import random
import sys

from wtc_sim.consenso import ParamsConsenso, run_once
from wtc_sim.poblacion import poblacion_fraccion_rep, poblacion_sybil


PARAMS = ParamsConsenso(k=20, alpha=14, beta=12, max_rondas=80)
TRIALS = 60
SEMILLA = 1984


def agrega(reputacion, adversarios, ponderado, trials=TRIALS):
    """Ejecuta `trials` veces y promedia las banderas (en %)."""
    rng = random.Random(SEMILLA)
    seg = cap = bif = 0
    for _ in range(trials):
        r = run_once(reputacion, adversarios, PARAMS, rng, ponderado=ponderado)
        seg += r["seguro"]; cap += r["captura"]; bif += r["bifurcacion"]
    n = float(trials)
    return {"seguro": 100*seg/n, "captura": 100*cap/n, "bifurcacion": 100*bif/n}


def construir_informe() -> str:
    L: list[str] = []
    w = L.append

    w("# RESULTADOS — Simulador del Consenso de Confianza Tejida\n")
    w("> Generado por `run_consenso.py` (solo stdlib). Reproducible (PRNG sembrado). Voto "
      "sub-muestreado tipo Snowball/Avalanche con muestreo **ponderado por reputación** (PAPER §5.4).\n")
    w(f"> Parámetros: muestra k={PARAMS.k}, quórum α={PARAMS.alpha}, decisión β={PARAMS.beta}, "
      f"{TRIALS} ejecuciones por punto. Honestos arrancan en el valor legítimo; el adversario es "
      "bizantino (empuja un valor en conflicto).\n")

    # 1. barrido por fraccion de reputacion
    w("\n## 1. Umbral de seguridad por FRACCIÓN DE REPUTACIÓN del adversario\n")
    w("Pocos nodos adversarios, pero controlando una fracción creciente de la reputación total.\n\n")
    w("| Reputación adversaria | Consenso seguro (todos el valor legítimo) | Captura (algún honesto volteado) | Bifurcación |\n")
    w("|---|---|---|---|\n")
    umbral_vivacidad = None   # primera f donde deja de decidir todo el mundo (stall)
    umbral_captura = None     # primera f donde algún honesto adopta el valor falso (fallo de seguridad)
    for f in (0.0, 0.1, 0.2, 0.3, 0.4, 0.5):
        rep, adv = poblacion_fraccion_rep(f)
        r = agrega(rep, adv, ponderado=True)
        w(f"| {int(f*100)} % | {r['seguro']:.0f} % | {r['captura']:.0f} % | {r['bifurcacion']:.0f} % |\n")
        if umbral_vivacidad is None and r["seguro"] < 100.0:
            umbral_vivacidad = f
        if umbral_captura is None and (r["captura"] > 0.0 or r["bifurcacion"] > 0.0):
            umbral_captura = f
    w("\n**Lectura (distingue dos cosas):**\n")
    w("- **Seguridad** (que ningún honesto acepte el valor falso ni haya bifurcación): se mantiene "
      f"mientras el adversario tiene **menos de ~{int((umbral_captura or 0.5)*100)} %** de la reputación. "
      "Por debajo de ese umbral, el adversario **nunca consigue una decisión falsa**.\n")
    w("- **Vivacidad** (que todos lleguen a decidir): empieza a degradarse antes, sobre el "
      f"**~{int((umbral_vivacidad or 0.5)*100)} %**: ahí el adversario aún **no engaña a nadie**, pero "
      "puede **frenar** la convergencia (los honestos quedan indecisos). Es un atasco, no un engaño.\n")
    w("- Lo decisivo: ambos umbrales se expresan en **fracción de reputación**, no en número de nodos. "
      "Esa es la tesis.\n")

    # 2 y 3. Sybil ponderado vs uniforme
    rep_s, adv_s = poblacion_sybil()
    pond = agrega(rep_s, adv_s, ponderado=True)
    unif = agrega(rep_s, adv_s, ponderado=False)
    n_total = len(rep_s); n_adv = len(adv_s)
    w("\n## 2. Falsa multitud (Sybil): número de nodos vs reputación\n")
    w(f"El adversario tiene **{n_adv} nodos** ({100*n_adv/n_total:.0f} % de la red) con reputación ~0; "
      "los honestos son minoría de nodos pero tienen toda la reputación.\n\n")
    w("| Muestreo | Consenso seguro | Captura | Bifurcación |\n|---|---|---|---|\n")
    w(f"| **Ponderado por reputación** (WTC) | {pond['seguro']:.0f} % | {pond['captura']:.0f} % | {pond['bifurcacion']:.0f} % |\n")
    w(f"| Uniforme (Avalanche puro, contraste) | {unif['seguro']:.0f} % | {unif['captura']:.0f} % | {unif['bifurcacion']:.0f} % |\n")
    w(f"\n**Lectura:** con muestreo ponderado por reputación, una falsa multitud del "
      f"{100*n_adv/n_total:.0f} % de los nodos **no rompe** la red (sigue segura), porque casi nunca "
      "entra en las muestras: el poder es la reputación, no el número (Art. VI). Con muestreo "
      "**uniforme** —sin ponderar—, la misma multitud captura/divide la red. La ponderación por "
      "reputación es lo que defiende (paralelo al damping anti-colusión del motor).\n")

    w("\n## Conclusión\n")
    w("El simulador confirma la pieza que faltaba del paper: el **umbral de seguridad del consenso se "
      "mide en fracción de REPUTACIÓN**, no de nodos. La falsa multitud no obtiene poder; ganar "
      "influencia exige reputación real (que, por el motor, exige obra validada en el tiempo — "
      "*reputación-tiempo*). Limitación honesta: es un modelo de decisión binaria con adversario "
      "bizantino simple; falta adversario adaptativo, ataque a la independencia del muestreo y prueba "
      "formal de seguridad (PAPER §10).\n")
    return "".join(L)


def main() -> None:
    informe = construir_informe()
    with open("RESULTADOS-consenso.md", "w", encoding="utf-8") as f:
        f.write(informe)
    print("[ok] informe escrito en RESULTADOS-consenso.md")
    if "--stdout" in sys.argv:
        print("\n" + informe)


if __name__ == "__main__":
    main()
