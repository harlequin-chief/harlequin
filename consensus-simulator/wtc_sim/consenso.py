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
    grupo: dict[str, int] | None = None,
    rondas_particion: int = 0,
    quorum_red: float = 0.0,
    perdida: float = 0.0,
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
    `grupo` + `rondas_particion`: PARTICIÓN de red. Durante las primeras `rondas_particion` rondas,
    cada nodo solo puede muestrear dentro de su propio grupo (la red está partida); después sana y se
    muestrea de toda la red. Mide safety (¿deciden distinto los dos lados?) y liveness (¿se recupera?).
    `quorum_red`: MITIGACIÓN anti-partición. Un nodo no FINALIZA (decide) si la reputación que alcanza
    a ver es < `quorum_red` del total. Bajo partición, el grupo aislado no llega al quórum -> no
    finaliza (se atasca, no forkea) -> recupera al sanar. 0.0 = sin mitigación (comportamiento base).
    `perdida`: prob. de que CADA respuesta consultada se pierda (latencia/pérdida de red). Reduce los
    votos efectivos por ronda -> convergencia más lenta (coste de liveness), pero el umbral α no cambia
    -> safety preservada. 0.0 = red fiable (comportamiento base).
    """
    ids = list(reputacion)
    if ponderado:
        pesos = [max(reputacion[i], 0.0) for i in ids]
    else:
        pesos = [1.0 for _ in ids]  # muestreo uniforme (contraste: ignora la reputación)
    cum = _cum_weights(pesos)

    usar_cap = cap_cluster is not None and clusters is not None

    # pools por grupo para la fase de partición (ids + cum_weights restringidos a cada grupo)
    pools: dict[int, tuple[list[str], list[float]]] = {}
    rep_grupo: dict[int, float] = {}
    if grupo is not None and rondas_particion > 0:
        for g in set(grupo.values()):
            gids = [i for i in ids if grupo.get(i) == g]
            gpesos = [max(reputacion[i], 0.0) if ponderado else 1.0 for i in gids]
            pools[g] = (gids, _cum_weights(gpesos))
            rep_grupo[g] = sum(gpesos)
    rep_total = sum(max(reputacion[i], 0.0) if ponderado else 1.0 for i in ids)

    def alcanza_quorum(nodo: str, ronda: int) -> bool:
        """¿El nodo ve suficiente reputación de la red para poder FINALIZAR? (mitigación partición)"""
        if quorum_red <= 0.0:
            return True
        visible = rep_grupo.get(grupo[nodo], rep_total) if (pools and ronda < rondas_particion) else rep_total
        return rep_total > 0 and visible / rep_total >= quorum_red

    def muestra(nodo: str, ronda: int) -> list[str]:
        """k nodos ponderados por reputación; con tope por clúster y/o partición de red si aplica."""
        # partición activa: el nodo solo ve su grupo
        if pools and ronda < rondas_particion:
            gids, gcum = pools[grupo[nodo]]
            base_ids, base_cum = gids, gcum
        else:
            base_ids, base_cum = ids, cum
        if not usar_cap:
            return rng.choices(base_ids, cum_weights=base_cum, k=params.k)
        ids_local, cum_local = base_ids, base_cum
        elegidos: list[str] = []
        por_cluster: dict[str, int] = {}
        intentos = 0
        limite = params.k * 40  # cota anti-bucle si no hay diversidad suficiente
        while len(elegidos) < params.k and intentos < limite:
            intentos += 1
            cand = rng.choices(ids_local, cum_weights=cum_local, k=1)[0]
            cl = clusters.get(cand, cand)
            if por_cluster.get(cl, 0) >= cap_cluster:
                continue
            elegidos.append(cand)
            por_cluster[cl] = por_cluster.get(cl, 0) + 1
        # si la diversidad no alcanza para k, se completa sin tope (no se penaliza la vivacidad)
        while len(elegidos) < params.k:
            elegidos.append(rng.choices(ids_local, cum_weights=cum_local, k=1)[0])
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

    for ronda in range(params.max_rondas):
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
            m = muestra(n, ronda)
            if perdida > 0.0:
                m = [s for s in m if rng.random() >= perdida]   # respuestas que se pierden
            unos = sum(1 for s in m if reporta(s) == 1)
            ceros = len(m) - unos
            color, cuenta = (1, unos) if unos >= ceros else (0, ceros)
            if cuenta >= params.alpha:
                if color == pref[n]:
                    racha[n] += 1
                else:
                    pref[n] = color
                    racha[n] = 1
                # finaliza solo con racha suficiente Y viendo quórum de la red (anti-partición)
                if racha[n] >= params.beta and alcanza_quorum(n, ronda):
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
