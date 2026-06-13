"""
Núcleo del consenso: voto sub-muestreado tipo Snowball (Avalanche) con muestreo ponderado por
reputación (SPEC §2.2; PAPER §5.4).

Modelo (decisión binaria, que basta para medir seguridad y bifurcación):
- Los nodos honestos arrancan prefiriendo el valor legítimo `0`.
- Los nodos adversarios son bizantinos: responden siempre `1` (empujan un valor en conflicto) e
  intentan voltear o dividir a los honestos.
- Cada ronda, cada nodo honesto no decidido consulta una muestra de `k` pares **ponderada por
  reputación**; si una mayoría de al menos `alpha` coincide en un color, refuerza su preferencia
  (Snowball); tras `beta` rondas consecutivas en el mismo color, decide.

La APORTACIÓN de WTC frente a Avalanche puro: el muestreo NO es uniforme. Un nodo se elige con
probabilidad proporcional a su reputación. Así, mil identidades con reputación ~0 casi nunca entran
en la muestra: el poder es la reputación, no el número (Art. VI).
"""

from __future__ import annotations

import itertools
import random
from dataclasses import dataclass


@dataclass
class ParamsConsenso:
    k: int = 20          # tamaño de la muestra por consulta
    alpha: int = 14      # quórum: nº mínimo que debe coincidir para contar (alpha > k/2)
    beta: int = 12       # rondas consecutivas en el mismo color para decidir
    max_rondas: int = 80


def _cum_weights(weights: list[float]) -> list[float]:
    return list(itertools.accumulate(weights))


def run_once(
    reputacion: dict[str, float],
    adversarios: set[str],
    params: ParamsConsenso,
    rng: random.Random,
    ponderado: bool = True,
) -> dict[str, int]:
    """
    Una ejecución del consenso. Devuelve un recuento de resultados entre los HONESTOS:
      - decididos_0: decidieron el valor legítimo (correcto)
      - decididos_1: decidieron el valor del adversario (capturados)
      - indecisos: no convergieron en max_rondas
    Más banderas agregadas: seguro (todos 0), captura (algún 1), bifurcacion (hay 0 y 1).
    """
    ids = list(reputacion)
    if ponderado:
        pesos = [max(reputacion[i], 0.0) for i in ids]
    else:
        pesos = [1.0 for _ in ids]  # muestreo uniforme (contraste: ignora la reputación)
    cum = _cum_weights(pesos)

    honestos = [i for i in ids if i not in adversarios]
    pref = {i: 0 for i in honestos}     # honestos arrancan en el valor legítimo
    racha = {i: 0 for i in honestos}
    decision: dict[str, int] = {}

    def reporta(i: str) -> int:
        if i in adversarios:
            return 1                    # bizantino: siempre empuja el valor en conflicto
        if i in decision:
            return decision[i]
        return pref[i]

    for _ in range(params.max_rondas):
        if len(decision) == len(honestos):
            break
        for n in honestos:
            if n in decision:
                continue
            muestra = rng.choices(ids, cum_weights=cum, k=params.k)
            unos = sum(1 for s in muestra if reporta(s) == 1)
            ceros = params.k - unos
            color, cuenta = (1, unos) if unos >= ceros else (0, ceros)
            if cuenta >= params.alpha:
                if color == pref[n]:
                    racha[n] += 1
                else:
                    pref[n] = color
                    racha[n] = 1
                if racha[n] >= params.beta:
                    decision[n] = color
            else:
                racha[n] = 0

    d0 = sum(1 for n in honestos if decision.get(n) == 0)
    d1 = sum(1 for n in honestos if decision.get(n) == 1)
    indecisos = len(honestos) - d0 - d1
    return {
        "decididos_0": d0,
        "decididos_1": d1,
        "indecisos": indecisos,
        "seguro": int(d1 == 0 and indecisos == 0),
        "captura": int(d1 > 0),
        "bifurcacion": int(d0 > 0 and d1 > 0),
    }
