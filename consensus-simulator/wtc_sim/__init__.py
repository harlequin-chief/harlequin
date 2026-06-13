"""
wtc_sim — Simulador del Consenso de Confianza Tejida (Woven Trust Consensus).

Mide, en cifras, la tesis del paper (`PAPER-confianza-tejida.md`): el poder en el consenso depende
de la REPUTACIÓN, no del número de nodos. Un atacante con muchas identidades pero poca reputación
(Sybil) no rompe la red; el umbral de seguridad se expresa en FRACCIÓN DE REPUTACIÓN del adversario,
no en fracción de nodos.

Implementa el voto sub-muestreado tipo Snowball/Avalanche (SPEC §2.2), con la aportación propia de
WTC: el muestreo se pondera por REPUTACIÓN (en vez de uniforme). Solo biblioteca estándar.
"""

from .consenso import run_once, ParamsConsenso
from .poblacion import poblacion_sybil, poblacion_fraccion_rep

__all__ = ["run_once", "ParamsConsenso", "poblacion_sybil", "poblacion_fraccion_rep"]
