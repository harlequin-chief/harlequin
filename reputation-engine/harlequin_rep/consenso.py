"""
Consenso: sorteo de comités ponderado por REPUTACIÓN, no por riqueza (SPEC §2.2, Art. VI).

Modela la selección de validadores estilo Algorand (sortition por VRF): cada época se elige un
comité rotatorio con probabilidad proporcional a la reputación (agregado conservador del vector,
§1.2b). Aquí el VRF se simula con un PRNG sembrado (determinista y reproducible); en producción
sería una función aleatoria verificable real.

Tesis a verificar (§2.4): crear nodos falsos NO da peso (nacen con reputación 0). El peso en el
consenso hereda la robustez del sistema de reputación.
"""

from __future__ import annotations

import random


def _peso_seleccion(rep_agregada: float, base: float, peso_base: float = 0.0) -> float:
    """
    Peso de un agente en el sorteo. Dominado por la reputación GANADA.

    `peso_base` permite, si se quiere, que la ciudadanía base aporte un mínimo (>0). Por defecto 0:
    el consenso lo gobierna la reputación ganada (puerta 2), no la mera ciudadanía (§1.4, Art. VI:
    "sin reputación ninguna voz se alza").
    """
    return max(0.0, rep_agregada) + peso_base * base


def sorteo_ponderado(
    pesos: dict[str, float],
    tam_comite: int,
    epocas: int = 1000,
    semilla: int = 1984,
) -> dict[str, int]:
    """
    Simula `epocas` sorteos de un comité de `tam_comite` miembros, sin reemplazo dentro de cada
    comité, con probabilidad proporcional a `pesos`. Devuelve id -> nº de veces seleccionado.

    Sirve para medir la CUOTA de comité que captura cada facción (honestos vs atacantes).
    """
    rng = random.Random(semilla)
    elegibles = [k for k, w in pesos.items() if w > 0]
    conteo: dict[str, int] = {k: 0 for k in pesos}

    tam = min(tam_comite, len(elegibles))
    if tam == 0:
        return conteo

    for _ in range(epocas):
        disponibles = elegibles.copy()
        w = {k: pesos[k] for k in disponibles}
        for _ in range(tam):
            total = sum(w[k] for k in disponibles)
            if total <= 0:
                break
            r = rng.uniform(0.0, total)
            acum = 0.0
            elegido = disponibles[-1]
            for k in disponibles:
                acum += w[k]
                if r <= acum:
                    elegido = k
                    break
            conteo[elegido] += 1
            disponibles.remove(elegido)

    return conteo
