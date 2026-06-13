#!/usr/bin/env python3
"""
Tests de autoauditoría del simulador de consenso. Sin dependencias (asserts planos).
Ejecutar:  python3 tests/test_consenso.py   (desde prototipos/consenso/)
"""

from __future__ import annotations

import os
import random
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from wtc_sim.consenso import ParamsConsenso, run_once
from wtc_sim.poblacion import poblacion_fraccion_rep, poblacion_sybil

PARAMS = ParamsConsenso(k=20, alpha=14, beta=12, max_rondas=80)


def _agrega(rep, adv, ponderado, trials=40, semilla=1984):
    rng = random.Random(semilla)
    seg = cap = 0
    for _ in range(trials):
        r = run_once(rep, adv, PARAMS, rng, ponderado=ponderado)
        seg += r["seguro"]; cap += r["captura"]
    return 100 * seg / trials, 100 * cap / trials


def test_sin_adversario_es_seguro():
    rep, adv = poblacion_fraccion_rep(0.0)
    seguro, captura = _agrega(rep, adv, ponderado=True)
    assert seguro == 100.0 and captura == 0.0


def test_sybil_ponderado_es_seguro():
    """93% de los nodos en manos del adversario, pero con reputación ~0 -> red segura."""
    rep, adv = poblacion_sybil()
    seguro, captura = _agrega(rep, adv, ponderado=True)
    assert seguro >= 95.0, f"sybil ponderado deberia ser seguro, fue {seguro}%"
    assert captura == 0.0


def test_sybil_uniforme_falla():
    """Sin ponderar por reputacion, la misma falsa multitud rompe la red (contraste)."""
    rep, adv = poblacion_sybil()
    seguro, captura = _agrega(rep, adv, ponderado=False)
    assert seguro < 50.0, f"sybil uniforme NO deberia ser seguro, fue {seguro}%"


def test_mayoria_reputacion_captura():
    """Con el 50% de la reputacion, el adversario logra capturar honestos."""
    rep, adv = poblacion_fraccion_rep(0.5)
    seguro, captura = _agrega(rep, adv, ponderado=True)
    assert captura > 0.0 and seguro == 0.0


def test_umbral_es_de_reputacion_no_de_nodos():
    """Poca reputacion adversaria (10%) en pocos nodos -> seguro; demuestra que pesa la reputacion."""
    rep, adv = poblacion_fraccion_rep(0.1)
    seguro, _ = _agrega(rep, adv, ponderado=True)
    assert seguro == 100.0


def main():
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    fallos = 0
    for t in tests:
        try:
            t(); print(f"  PASS  {t.__name__}")
        except AssertionError as e:
            fallos += 1; print(f"  FAIL  {t.__name__}: {e}")
    print(f"\n{len(tests)-fallos}/{len(tests)} tests OK")
    sys.exit(1 if fallos else 0)


if __name__ == "__main__":
    main()
