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
from wtc_sim.poblacion import (
    poblacion_adversario_agrupado,
    poblacion_fraccion_rep,
    poblacion_sybil,
)

PARAMS = ParamsConsenso(k=20, alpha=14, beta=12, max_rondas=80)


def _agrega(rep, adv, ponderado, trials=40, semilla=1984):
    rng = random.Random(semilla)
    seg = cap = 0
    for _ in range(trials):
        r = run_once(rep, adv, PARAMS, rng, ponderado=ponderado)
        seg += r["seguro"]; cap += r["captura"]
    return 100 * seg / trials, 100 * cap / trials


def _agrega_cap(rep, adv, clusters, cap_cluster, adversario="fijo", trials=60, semilla=1984):
    rng = random.Random(semilla)
    seg = cap = 0
    for _ in range(trials):
        r = run_once(rep, adv, PARAMS, rng, ponderado=True,
                     clusters=clusters, cap_cluster=cap_cluster, adversario=adversario)
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


def test_independencia_protege_de_bloque_correlacionado():
    """
    Un adversario con 45% de la reputación pero TODA en un clúster correlacionado captura la red con
    muestreo rep-only; el muestreo ponderado por INDEPENDENCIA (tope por clúster) lo neutraliza.
    """
    rep, adv, cl = poblacion_adversario_agrupado(0.45, n_clusters_adv=1)
    seg_sin, cap_sin = _agrega_cap(rep, adv, cl, cap_cluster=None)      # rep-only
    seg_cap, cap_cap = _agrega_cap(rep, adv, cl, cap_cluster=3)         # +independencia
    assert cap_sin > 0.0, "rep-only deberia ser capturado por el bloque correlacionado"
    assert seg_cap >= 95.0 and cap_cap == 0.0, "el tope por independencia deberia proteger"


def test_independencia_cede_si_el_adversario_fragmenta_lo_suficiente():
    """
    El tope (cap) sobre k=20, alpha=14 exige al adversario >= ceil(alpha/cap) clústeres distintos
    para capturar. Con cap=3 hace falta fragmentar en >=5 bloques; con menos, la red sigue segura.
    Frontera honesta: cada bloque tiene que parecer INDEPENDIENTE (lo que el motor de reputación
    resiste).
    """
    seg2, cap2 = _agrega_cap(*poblacion_adversario_agrupado(0.45, n_clusters_adv=2), cap_cluster=3)
    seg6, cap6 = _agrega_cap(*poblacion_adversario_agrupado(0.45, n_clusters_adv=6), cap_cluster=3)
    assert cap2 == 0.0, "con 2 bloques (<5) el tope deberia aguantar"
    assert cap6 > 0.0, "con 6 bloques (>=5) el adversario deberia poder capturar de nuevo"


def test_adversario_adaptativo_no_rompe_seguridad():
    """
    El adversario ADAPTATIVO (divisor anti-finalidad) ataca la VIVACIDAD (atasca), pero por debajo
    del umbral de reputación NUNCA fuerza una decisión falsa (seguridad intacta).
    """
    rep, adv, cl = poblacion_adversario_agrupado(0.3, n_clusters_adv=1)
    _, captura = _agrega_cap(rep, adv, cl, cap_cluster=None, adversario="adaptativo")
    assert captura == 0.0, "el adaptativo no deberia capturar (solo atascar)"


def test_perdida_degrada_liveness_no_safety():
    """
    Pérdida de mensajes (latencia/red poco fiable): un adversario por debajo del umbral (25%) NUNCA
    captura aunque la pérdida sea alta (safety preservada); la pérdida solo añade atasco (liveness).
    """
    rep, adv = poblacion_fraccion_rep(0.25)
    for perd in (0.0, 0.4, 0.6):
        rng = random.Random(1984)
        cap = 0
        for _ in range(40):
            r = run_once(rep, adv, PARAMS, rng, ponderado=True, perdida=perd)
            cap += r["captura"]
        assert cap == 0, f"pérdida {perd}: no debería capturar (safety), fue {cap}"


def test_particion_larga_forkea_sin_mitigacion():
    """
    Ataque de partición: un adversario inofensivo global (15%) concentrado en el grupo pequeño, con
    una partición larga, captura ese grupo y produce FORK al sanar (fallo de safety). Documenta el
    riesgo real que el simulador encontró.
    """
    import particion
    m = particion.medir(0.15, D=90, quorum_red=0.0, trials=40)
    assert m["fork"] > 50.0, f"la partición larga debería forkear, fue {m['fork']}%"


def test_particion_quorum_preserva_safety():
    """
    Mitigación quórum-de-red: condicionar la finalidad a ver ≥60% de la reputación elimina casi por
    completo el fork (safety preservada), a cambio de atasco (liveness recuperable).
    """
    import particion
    sin = particion.medir(0.15, D=90, quorum_red=0.0, trials=40)
    con = particion.medir(0.15, D=90, quorum_red=0.6, trials=40)
    assert con["fork"] < sin["fork"] * 0.2, f"el quórum debería recortar el fork drásticamente ({sin['fork']}→{con['fork']})"
    assert con["fork"] < 10.0, f"con quórum el fork debería ser bajo, fue {con['fork']}%"


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
