"""
Construcción de poblaciones de nodos para el simulador de consenso.

Dos escenarios clave:
- `poblacion_fraccion_rep`: el adversario controla una FRACCIÓN de la reputación total (pocos nodos,
  mucha reputación). Sirve para barrer el umbral de seguridad en función de la reputación adversaria.
- `poblacion_sybil`: el adversario tiene MUCHÍSIMOS nodos pero reputación ~0 (la falsa multitud).
  Sirve para mostrar que el número de nodos no da poder si la reputación es nula.
"""

from __future__ import annotations

import random


def poblacion_fraccion_rep(
    f: float,
    n_honestos: int = 80,
    n_adversarios: int = 5,
) -> tuple[dict[str, float], set[str]]:
    """
    El adversario controla una fracción `f` (0..1) de la reputación TOTAL, repartida entre
    `n_adversarios` nodos. Los honestos tienen reputación 1 cada uno.
    """
    reputacion: dict[str, float] = {}
    for i in range(n_honestos):
        reputacion[f"h{i}"] = 1.0
    total_honesto = float(n_honestos)

    adversarios: set[str] = set()
    if f > 0.0:
        # share adversario = f  =>  adv_total / (adv_total + total_honesto) = f
        adv_total = f * total_honesto / (1.0 - f)
        por_nodo = adv_total / n_adversarios
        for i in range(n_adversarios):
            aid = f"a{i}"
            reputacion[aid] = por_nodo
            adversarios.add(aid)
    return reputacion, adversarios


def poblacion_adversario_agrupado(
    f: float,
    n_honestos: int = 80,
    n_adversarios: int = 12,
    honestos_por_cluster: int = 2,
    n_clusters_adv: int = 1,
) -> tuple[dict[str, float], set[str], dict[str, str]]:
    """
    Adversario CORRELACIONADO: controla una fracción `f` de la reputación total, repartida entre
    `n_clusters_adv` clústeres de confianza. Con `n_clusters_adv=1` toda su reputación vive en UN
    bloque (caso correlacionado puro). Subiendo `n_clusters_adv` el adversario FRAGMENTA su bloque
    para evadir el tope por clúster del muestreo por independencia (la frontera honesta: fragmentar
    exige que cada sub-bloque parezca un clúster independiente, justo lo que el motor de reputación
    —detección de comunidades + damping— está diseñado para resistir).

    Los honestos se reparten en muchos clústeres pequeños e independientes (`honestos_por_cluster`).
    Devuelve (reputacion, adversarios, clusters).
    """
    reputacion: dict[str, float] = {}
    clusters: dict[str, str] = {}
    for i in range(n_honestos):
        hid = f"h{i}"
        reputacion[hid] = 1.0
        clusters[hid] = f"hc{i // max(1, honestos_por_cluster)}"   # clústeres honestos pequeños
    total_honesto = float(n_honestos)

    adversarios: set[str] = set()
    if f > 0.0:
        adv_total = f * total_honesto / (1.0 - f)
        por_nodo = adv_total / n_adversarios
        nclu = max(1, n_clusters_adv)
        for i in range(n_adversarios):
            aid = f"a{i}"
            reputacion[aid] = por_nodo
            clusters[aid] = f"adv{i % nclu}"   # repartidos en n_clusters_adv bloques
            adversarios.add(aid)
    return reputacion, adversarios, clusters


def poblacion_sybil(
    n_honestos: int = 80,
    n_sybil: int = 1000,
    rep_sybil: float = 1e-6,
) -> tuple[dict[str, float], set[str]]:
    """
    Falsa multitud: `n_sybil` nodos adversarios con reputación ~0 (nacen sin reputación, §1.5),
    frente a `n_honestos` honestos con reputación 1. El adversario es la GRAN MAYORÍA de nodos.
    """
    reputacion: dict[str, float] = {}
    for i in range(n_honestos):
        reputacion[f"h{i}"] = 1.0
    adversarios: set[str] = set()
    for i in range(n_sybil):
        sid = f"s{i}"
        reputacion[sid] = rep_sybil
        adversarios.add(sid)
    return reputacion, adversarios
