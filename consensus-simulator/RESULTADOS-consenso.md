# RESULTADOS — Simulador del Consenso de Confianza Tejida
> Generado por `run_consenso.py` (solo stdlib). Reproducible (PRNG sembrado). Voto sub-muestreado tipo Snowball/Avalanche con muestreo **ponderado por reputación** (PAPER §5.4).
> Parámetros: muestra k=20, quórum α=14, decisión β=12, 60 ejecuciones por punto. Honestos arrancan en el valor legítimo; el adversario es bizantino (empuja un valor en conflicto).

## 1. Umbral de seguridad por FRACCIÓN DE REPUTACIÓN del adversario
Pocos nodos adversarios, pero controlando una fracción creciente de la reputación total.

| Reputación adversaria | Consenso seguro (todos el valor legítimo) | Captura (algún honesto volteado) | Bifurcación |
|---|---|---|---|
| 0 % | 100 % | 0 % | 0 % |
| 10 % | 100 % | 0 % | 0 % |
| 20 % | 50 % | 0 % | 0 % |
| 30 % | 0 % | 0 % | 0 % |
| 40 % | 0 % | 97 % | 0 % |
| 50 % | 0 % | 100 % | 0 % |

**Lectura (distingue dos cosas):**
- **Seguridad** (que ningún honesto acepte el valor falso ni haya bifurcación): se mantiene mientras el adversario tiene **menos de ~40 %** de la reputación. Por debajo de ese umbral, el adversario **nunca consigue una decisión falsa**.
- **Vivacidad** (que todos lleguen a decidir): empieza a degradarse antes, sobre el **~20 %**: ahí el adversario aún **no engaña a nadie**, pero puede **frenar** la convergencia (los honestos quedan indecisos). Es un atasco, no un engaño.
- Lo decisivo: ambos umbrales se expresan en **fracción de reputación**, no en número de nodos. Esa es la tesis.

## 2. Falsa multitud (Sybil): número de nodos vs reputación
El adversario tiene **1000 nodos** (93 % de la red) con reputación ~0; los honestos son minoría de nodos pero tienen toda la reputación.

| Muestreo | Consenso seguro | Captura | Bifurcación |
|---|---|---|---|
| **Ponderado por reputación** (WTC) | 100 % | 0 % | 0 % |
| Uniforme (Avalanche puro, contraste) | 0 % | 100 % | 0 % |

**Lectura:** con muestreo ponderado por reputación, una falsa multitud del 93 % de los nodos **no rompe** la red (sigue segura), porque casi nunca entra en las muestras: el poder es la reputación, no el número (Art. VI). Con muestreo **uniforme** —sin ponderar—, la misma multitud captura/divide la red. La ponderación por reputación es lo que defiende (paralelo al damping anti-colusión del motor).

## Conclusión
El simulador confirma la pieza que faltaba del paper: el **umbral de seguridad del consenso se mide en fracción de REPUTACIÓN**, no de nodos. La falsa multitud no obtiene poder; ganar influencia exige reputación real (que, por el motor, exige obra validada en el tiempo — *reputación-tiempo*). Limitación honesta: es un modelo de decisión binaria con adversario bizantino simple; falta adversario adaptativo, ataque a la independencia del muestreo y prueba formal de seguridad (PAPER §10).
