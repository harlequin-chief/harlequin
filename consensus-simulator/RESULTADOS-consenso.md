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

## 3. Muestreo por INDEPENDENCIA contra un adversario correlacionado (PAPER §5.4)
Ahora el adversario es más fino: no es una multitud sin reputación, sino un **bloque que SÍ ganó reputación**, pero toda **correlacionada** (un solo clúster de confianza que se avaló entre sí). El muestreo ponderado solo por reputación lo trata como si fuera independiente. La defensa de WTC: ponderar también por **independencia**, limitando a `cap` los nodos que un mismo clúster puede aportar a cada comité de k=20 (α=14).

| Reputación adversaria (1 clúster) | rep-only: seguro / captura | +independencia (cap=3): seguro / captura |
|---|---|---|
| 30 % | 0 % / 0 % | 100 % / 0 % |
| 40 % | 0 % / 97 % | 100 % / 0 % |
| 45 % | 0 % / 100 % | 100 % / 0 % |
| 50 % | 0 % / 100 % | 100 % / 0 % |

**Lectura:** con muestreo rep-only, un bloque correlacionado con ≥~40 % de la reputación **captura** la red. El muestreo por independencia (tope por clúster) lo **neutraliza**: el bloque no puede ocupar suficientes asientos del comité aunque tenga la reputación. Estructuralmente, para forzar el color falso necesita ⌈α/cap⌉ asientos de clústeres distintos.

**Frontera honesta — el adversario fragmenta su bloque** (45 % de reputación, cap=3):

| Clústeres del adversario | +independencia (cap=3): seguro / captura |
|---|---|
| 1 | 100 % / 0 % |
| 2 | 100 % / 0 % |
| 3 | 0 % / 0 % |
| 4 | 0 % / 0 % |
| 5 | 0 % / 93 % |
| 6 | 0 % / 100 % |

La protección aguanta hasta que el adversario fragmenta en **⌈α/cap⌉ = ⌈14/3⌉ = 5** clústeres distintos; con 5+ vuelve a capturar. Pero fragmentar exige que **cada** sub-bloque parezca un clúster **independiente** ante la detección de comunidades — justo lo que el **motor de reputación** (damping + comunidades) está diseñado para resistir, y que ya mostramos hace que el lavado rinda **0** poder de consenso. *Los dos prototipos componen:* el muestreo por independencia del consenso descansa en la identificación de clústeres que aporta el motor.

## 4. Adversario ADAPTATIVO (divisor anti-finalidad)
En vez de empujar siempre el mismo valor, el adversario reporta cada ronda el color **minoritario** entre los honestos para mantenerlos divididos e impedir que cualquier color cuaje (worst-case: ve el estado del momento). Mide si puede romper la SEGURIDAD o solo la VIVACIDAD.

| Reputación adversaria (1 clúster) | adaptativo, rep-only: seguro / captura / atasco | adaptativo, +indep cap=3 |
|---|---|---|
| 20 % | 50 % / 0 % / 50 % | 100 % / 0 % / 0 % |
| 30 % | 0 % / 0 % / 100 % | 100 % / 0 % / 0 % |
| 40 % | 0 % / 0 % / 100 % | 100 % / 0 % / 0 % |
| 50 % | 0 % / 0 % / 100 % | 100 % / 0 % / 0 % |

**Lectura:** el adversario adaptativo **nunca fuerza una decisión falsa** (captura 0 %): ataca la **vivacidad** (atasca la convergencia), no la **seguridad**. Y bajo muestreo por independencia, ni siquiera atasca: su bloque correlacionado no entra lo bastante en los comités. Coherente con Snowball/Avalanche: la seguridad es robusta; el coste adaptativo es liveness, recuperable.

## 5. Partición de red: ataque de partición + mitigación quórum-de-red
Se rompe el supuesto de red completa: los honestos se parten en un grupo grande A (60) y uno pequeño B (20); un adversario inofensivo global (15%) concentra su reputación en B. La red está partida `D` rondas (cada grupo solo se ve a sí mismo) y luego sana.

| D (rondas partido) | fork (sin mitig.) | fork (+quórum 60%) | atasco (+quórum) |
|---:|---:|---:|---:|
| 0 | 0 % | 0 % | 0 % |
| 20 | 40 % | 0 % | 0 % |
| 50 | 97 % | 2 % | 0 % |
| 90 | 100 % | 2 % | 33 % |

**Hallazgo (honesto, importante):** una partición larga **concentra la cuota LOCAL del adversario** en el grupo pequeño por encima del umbral → B finaliza el valor falso → al sanar, **FORK**. Un 15% global inofensivo se vuelve **100% de fork** con partición de 90 rondas. **Es un fallo de safety real**, encontrado en simulación antes de construir la cadena.

**Mitigación:** condicionar la **finalidad** a ver un **quórum de la reputación total** (un nodo no decide si alcanza a ver <60% de la red). Bajo partición, B (25% de la rep) no llega al quórum → **no finaliza, se atasca** (coste de liveness) en vez de forkear, y **recupera al sanar**. El fork cae de ~100% a ~0–2%. Es la regla esperable de un BFT robusto: **ante duda de partición, preferir parar (liveness) antes que decidir mal (safety)**. Pendiente: detección dinámica de la fracción vista (aquí se modela con el grupo) y latencia/pérdida de mensajes.

## 6. Pérdida de mensajes: latencia / red poco fiable
Cada respuesta consultada se pierde con probabilidad `p` (modela latencia y pérdida de red). Mide si la red poco fiable rompe la seguridad o solo ralentiza.

| pérdida p | sin adversario: seguro / atasco | adversario 25%: captura / atasco |
|---:|---:|---:|
| 0 % | 100 % / 0 % | 0 % / 100 % |
| 20 % | 53 % / 47 % | 0 % / 100 % |
| 40 % | 0 % / 100 % | 0 % / 100 % |
| 60 % | 0 % / 100 % | 0 % / 100 % |

**Lectura:** la pérdida degrada la **vivacidad** (más atasco) pero **nunca rompe la seguridad** (captura 0 % a cualquier pérdida): el umbral α no cambia. Implicación de parámetro: para progresar bajo pérdida `p` hacen falta suficientes respuestas vivas, **α ≤ k·(1−p)**; con k=20, α=14 se tolera pérdida hasta ~30 % antes de atascarse del todo. La elección de α/k acota la pérdida tolerable — un parámetro a fijar con el modelado de red real.

## Conclusión
El simulador confirma y endurece las piezas del paper: (1) el **umbral de seguridad se mide en fracción de REPUTACIÓN**, no de nodos; (2) el **muestreo por independencia** (PAPER §5.4) eleva ese umbral frente a un adversario **correlacionado** — vencerlo exige fragmentar en clústeres que parezcan independientes, lo que el motor de reputación resiste (*los dos prototipos componen*); (3) un adversario **adaptativo** solo ataca la **vivacidad**, nunca la seguridad; (4) bajo **partición** aparece un fork real si se finaliza a ciegas, **mitigado** condicionando la finalidad a un quórum de red (safety sobre liveness). Limitación honesta restante: modelo binario, latencia/pérdida de mensajes no modeladas, y prueba **formal** de safety/liveness (PAPER §10).
