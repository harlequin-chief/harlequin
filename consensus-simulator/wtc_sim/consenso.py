"""
Núcleo del consenso: voto sub-muestreado tipo Snowball (Avalanche) con muestreo ponderado por
reputación (SPEC §2.2; PAPER §5.4).

Modelo (decisión binaria, que basta para medir seguridad y bifurcación):
- Los nodos honestos arrancan prefiriendo el valor legítimo `0`.
- Los nodos adversarios son bizantinos. Dos estrategias:
    * "fijo": responden siempre `1` (empujan un valor en conflicto).
    * "adaptativo": cada ronda reportan el color que MENOS apoyo tiene entre los honestos, para
      mantenerlos divididos e impedir que cualquier color alcance la racha de decisión (ataque a la
      VIVACIDAD / anti-finalidad). Worst-case: ven el estado honesto del momento.
- Cada ronda, cada nodo honesto no decidido consulta una muestra de `k` pares **ponderada por
  reputación**; si una mayoría de al menos `alpha` coincide en un color, refuerza su preferencia
  (Snowball); tras `beta` rondas consecutivas en el mismo color, decide.

DOS aportaciones de WTC frente a Avalanche puro (ambas conectan con el motor de reputación):
1. El muestreo NO es uniforme: un nodo entra con probabilidad ∝ su reputación. Mil identidades con
   reputación ~0 casi nunca entran en la muestra (el poder es la reputación, no el número, Art. VI).
2. Muestreo ponderado por INDEPENDENCIA (PAPER §5.4): el comité se fuerza a ser DIVERSO limitando
   cuántos nodos puede aportar un mismo clúster de confianza (`cap_cluster`). Así, un adversario que
   concentró mucha reputación en un solo bloque correlacionado NO puede copar la muestra: su
   influencia queda acotada por estructura, no solo por su reputación. Defiende contra fallos
   correlacionados (toda una vecindad de confianza mintiendo a la vez).
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
    clusters: dict[str, str] | None = None,
    cap_cluster: int | None = None,
    adversario: str = "fijo",
) -> dict[str, int]:
    """
    Una ejecución del consenso. Devuelve un recuento de resultados entre los HONESTOS:
      - decididos_0: decidieron el valor legítimo (correcto)
      - decididos_1: decidieron el valor del adversario (capturados)
      - indecisos: no convergieron en max_rondas
    Más banderas agregadas: seguro (todos 0), captura (algún 1), bifurcacion (hay 0 y 1).

    `cap_cluster`: si se da (con `clusters`), ninguna muestra puede contener más de `cap_cluster`
    nodos del mismo clúster -> muestreo ponderado por independencia (PAPER §5.4).
    `adversario`: "fijo" (siempre 1) o "adaptativo" (reporta el color minoritario entre honestos).
    """
    ids = list(reputacion)
    if ponderado:
        pesos = [max(reputacion[i], 0.0) for i in ids]
    else:
        pesos = [1.0 for _ in ids]  # muestreo uniforme (contraste: ignora la reputación)
    cum = _cum_weights(pesos)

    usar_cap = cap_cluster is not None and clusters is not None

    def muestra() -> list[str]:
        """k nodos ponderados por reputación; con tope por clúster si se pide (independencia)."""
        if not usar_cap:
            return rng.choices(ids, cum_weights=cum, k=params.k)
        elegidos: list[str] = []
        por_cluster: dict[str, int] = {}
        intentos = 0
        limite = params.k * 40  # cota anti-bucle si no hay diversidad suficiente
        while len(elegidos) < params.k and intentos < limite:
            intentos += 1
            cand = rng.choices(ids, cum_weights=cum, k=1)[0]
            cl = clusters.get(cand, cand)
            if por_cluster.get(cl, 0) >= cap_cluster:
                continue
            elegidos.append(cand)
            por_cluster[cl] = por_cluster.get(cl, 0) + 1
        # si la diversidad no alcanza para k, se completa sin tope (no se penaliza la vivacidad)
        while len(elegidos) < params.k:
            elegidos.append(rng.choices(ids, cum_weights=cum, k=1)[0])
        return elegidos

    honestos = [i for i in ids if i not in adversarios]
    pref = {i: 0 for i in honestos}     # honestos arrancan en el valor legítimo
    racha = {i: 0 for i in honestos}
    decision: dict[str, int] = {}

    color_adv = 1  # se recalcula por ronda si el adversario es adaptativo

    def reporta(i: str) -> int:
        if i in adversarios:
            return color_adv
        if i in decision:
            return decision[i]
        return pref[i]

    for _ in range(params.max_rondas):
        if len(decision) == len(honestos):
            break
        if adversario == "adaptativo":
            # color minoritario entre la preferencia honesta actual -> mantener la red dividida
            estado = [decision.get(n, pref[n]) for n in honestos]
            unos = sum(1 for c in estado if c == 1)
            ceros = len(estado) - unos
            color_adv = 1 if unos <= ceros else 0
        for n in honestos:
            if n in decision:
                continue
            m = muestra()
            unos = sum(1 for s in m if reporta(s) == 1)
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
