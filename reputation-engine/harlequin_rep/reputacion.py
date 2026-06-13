"""
Cálculo de la reputación vectorial (el núcleo, SPEC §1).

Idea central (§1.3, §1.5, §2.4): la reputación NO se fabrica solo avalándose entre cuentas. Se
**ancla en evidencia real verificable** (tratos liquidados, trabajo comprobado) y se propaga por
el grafo de avales, pero esa propagación está (a) anclada al pre-trust de evidencia y (b)
amortiguada anti-colusión (§1.6). Resultado esperado:
  - Sybils sin evidencia ni avales de reputados  -> reputación ~ 0  (poder ~ 0).
  - Anillo de colusión que se avala en círculo    -> reputación ~ 0  (damping + sin ancla).
  - Honestos con evidencia + avales independientes -> reputación alta.

Algoritmo: EigenTrust (Kamvar et al. 2003) con teletransporte al pre-trust.
    t_{k+1} = (1 - alpha) * C^T t_k  +  alpha * p
donde:
  - C = matriz de confianza local fila-estocástica, con damping anti-colusión (graph.py).
  - p = pre-trust = evidencia objetiva normalizada (+ semilla génesis). Es el ANCLA real.
  - alpha = peso del ancla (cuánto se reinyecta hacia la evidencia en cada paso).
  - la masa "colgante" (nodos que no avalan a nadie) se reparte según p, no uniforme -> refuerza
    el anclaje en evidencia.
"""

from __future__ import annotations

from .graph import GrafoConfianza
from .model import DIMENSIONES, Agente


def _pretrust(agentes: list[Agente], dim: str, peso_genesis: float = 1.0) -> dict[str, float]:
    """
    Pre-trust p por dimensión: evidencia objetiva normalizada (§1.3a) + semilla génesis (§1.4).

    La cohorte génesis recibe una pequeña semilla (andamiaje temporal, diseñado para diluirse
    al crecer la red). Todo lo demás del ancla viene de evidencia real por dimensión.
    Si no hubiese NADA de ancla, se cae a uniforme entre humanos únicos (degenerado, evita /0).
    """
    bruto: dict[str, float] = {}
    for a in agentes:
        ev = a.evidencia_dim(dim)
        semilla = peso_genesis if a.tipo.value == "genesis" else 0.0
        bruto[a.id] = ev + semilla

    total = sum(bruto.values())
    if total <= 0:
        humanos = [a for a in agentes if a.es_humano_unico]
        if not humanos:
            return {a.id: 0.0 for a in agentes}
        return {a.id: (1.0 / len(humanos) if a.es_humano_unico else 0.0) for a in agentes}
    return {k: v / total for k, v in bruto.items()}


def reputacion_dimension(
    agentes: list[Agente],
    grafo: GrafoConfianza,
    dim: str,
    alpha: float = 0.30,
    iteraciones: int = 200,
    tol: float = 1e-12,
    escala: float = 1000.0,
    damping: bool = True,
) -> dict[str, float]:
    """
    Reputación GANADA (puerta 2, §1.4) de cada agente en una dimensión.

    Devuelve un dict id -> reputación (escalada a `escala` para legibilidad; el reparto es lo que
    importa, no las unidades). NO incluye la ciudadanía base (§1.4), que se suma aparte donde
    proceda (la base es 1 por persona y no se gana).
    """
    nodos = [a.id for a in agentes]
    p = _pretrust(agentes, dim)
    C = grafo.matriz_local_amortiguada(dim, nodos, damping=damping)

    # Suma de cada fila de C (≤ 1). El déficit (1 - suma) es la masa que NO se propaga: o bien el
    # nodo no avala a nadie (colgante, suma 0) o sus avales son endogámicos y se amortiguaron (§1.6).
    # Esa masa se reinyecta hacia el pre-trust (anclaje en evidencia). Conserva la masa total = 1.
    suma_fila = {i: sum(C[i].values()) for i in nodos}

    t = dict(p)  # arranque en el pre-trust
    for _ in range(iteraciones):
        nt = {n: alpha * p[n] for n in nodos}
        fuga_total = 0.0

        for i in nodos:
            ti = t[i]
            if ti == 0.0:
                continue
            emitido = (1.0 - alpha) * ti
            fila = C[i]
            for j, w in fila.items():
                nt[j] += emitido * w
            # déficit de fila -> se fuga al pre-trust
            fuga_total += emitido * (1.0 - suma_fila[i])

        if fuga_total:
            for n in nodos:
                nt[n] += fuga_total * p[n]

        # convergencia
        delta = sum(abs(nt[n] - t[n]) for n in nodos)
        t = nt
        if delta < tol:
            break

    return {k: v * escala for k, v in t.items()}


def reputacion_vectorial(
    agentes: list[Agente],
    grafo: GrafoConfianza,
    **kwargs,
) -> dict[str, dict[str, float]]:
    """Reputación ganada por agente, como VECTOR sobre todas las dimensiones (§1.2b)."""
    por_dim = {dim: reputacion_dimension(agentes, grafo, dim, **kwargs) for dim in DIMENSIONES}
    salida: dict[str, dict[str, float]] = {}
    for a in agentes:
        salida[a.id] = {dim: por_dim[dim][a.id] for dim in DIMENSIONES}
    return salida


def agregado_conservador(vector: dict[str, float], modo: str = "min") -> float:
    """
    Agregación CONSERVADORA del vector (§1.2b): mínimos o medianas, NUNCA suma.

    Para poderes que exigen fiabilidad global (consenso, aval), una dimensión alta NO compensa una
    baja: no se "compra" integridad con pericia. Por defecto `min` (el más conservador).
    """
    vals = list(vector.values())
    if not vals:
        return 0.0
    if modo == "min":
        return min(vals)
    if modo == "mediana":
        from statistics import median

        return median(vals)
    if modo == "media":
        return sum(vals) / len(vals)
    raise ValueError(f"modo desconocido: {modo}")


def decaer(reputacion: dict[str, float], factor: float = 0.9) -> dict[str, float]:
    """
    Decaimiento por inactividad (§1.7): la reputación no contribuida se evapora.

    Modelo simple por época: r <- r * factor para quien no aportó evidencia nueva. Granjear y
    luego quedarse quieto no rinde a largo plazo (defensa anti-colusión adicional, §1.6).
    """
    return {k: v * factor for k, v in reputacion.items()}
