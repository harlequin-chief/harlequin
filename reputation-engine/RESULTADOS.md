# RESULTADOS — Prototipo del motor de reputación de Harlequin
> Generado por `run_all.py` (solo stdlib). Reproducible: PRNG sembrado. Las cifras son
> **relativas** (reparto de poder), no unidades absolutas. Mapea SPEC §1 (reputación), §1.6
> (anti-colusión), §2.2 (consenso) y §5 (salida/seudónimo).

## Tesis que se pone a prueba
**El muro anti-Sybil real es la reputación GANADA, no la prueba de personalidad** (SPEC §1.5,
§2.4). Crear identidades falsas o granjear avales en círculo NO debe dar poder estructural,
porque la reputación se ancla en evidencia real y se amortigua la colusión (§1.6).

## 1. Reputación y poder de consenso por escenario
| Escenario | Nº agentes | Cuota de reputación ganada | Cuota de comité de consenso |
|---|---|---|---|
| **honesto** | 45 | genesis=8.47%, honesto=91.53% | genesis=9.75%, honesto=90.25% |
| **sybil** | 245 | genesis=8.47%, honesto=91.53% | genesis=9.75%, honesto=90.25% |
| **colusion** | 75 | genesis=8.05%, honesto=88.01%, colusor=3.94% | genesis=9.94%, honesto=90.06% |

**Lectura:**
- **Sybil:** 200 cuentas falsas (40% de la red) capturan **0.00%** de la reputación y **0.00%** del comité de consenso. Nacen con reputación 0; sin evidencia ni avales de reputados, no obtienen poder.
- **Colusión:** el anillo de 30 que se avala en círculo captura **3.94%** de la reputación y **0.00%** del comité, pese a los 3 honestos engañados que lo avalan.

## 2. ¿Cuánto aporta el anti-colusión (damping, §1.6)?
Ataque de **lavado de reputación**: un miembro del anillo (`c0`) SÍ tiene reputación legítima
alta (trabajo real) e intenta **repartirla** a 29 secuaces avalándose en círculo. El damping
debe impedir la propagación: `c0` conserva lo suyo, los secuaces se quedan en ~0.

| | Reputación de `c0` (legítima) | Repartida a los 29 secuaces | Máx. de un secuaz |
|---|---|---|---|
| **SIN damping** | 83.7 | 213.9 | 17.5 |
| **CON damping** | 133.2 | 24.6 | 12.7 |

El damping recorta la reputación lavada a los secuaces **8.7×**.
Sin el anti-colusión, un solo miembro con reputación real puede 'prestársela' a todo su
anillo de títeres; con él, la reputación se queda donde se ganó.

## 3. Blanqueo de seudónimo (whitewashing, §5)
Mismo humano, dos máscaras: una consolidada y una nueva.

| Seudónimo | Reputación ganada |
|---|---|
| consolidado (`blanq_viejo`) | 59.23 |
| nuevo (`blanq_nuevo`) | 0.00 |

Abandonar el seudónimo para 'empezar limpio' cuesta **toda** la reputación ganada: vuelve a
la ciudadanía base. La reputación se ata a la máscara y no se transfiere (Art. VII/VIII).

## 4. Responsabilidad persistente + slashing en cascada (§1.5c, §1.7)
El ahijado defrauda y pierde 100. El golpe sube por la cadena de avales (½ por salto).

| Agente | Antes | Después | Δ |
|---|---|---|---|
| ahijado | 100.0 | 0.0 | -100.0 |
| intermedio | 120.0 | 70.0 | -50.0 |
| mentor | 200.0 | 175.0 | -25.0 |
| ajeno | 150.0 | 150.0 | +0.0 |

Quien avala responde de a quién metió, aunque pase el tiempo. El `ajeno` (no avaló) no se
ve afectado. Apadrinar a la ligera sale caro -> fuerza selectividad.

## 5. Economía de avales (§1.5c)
**Cupo de avales vivos = función sublineal de la reputación** (rendimientos decrecientes):

| Reputación | Cupo de avales |
|---|---|
| 1 | 3 |
| 10 | 10 |
| 100 | 19 |
| 1000 | 29 |
| 10000 | 39 |

**Dividendo de mentor:** apadrinar a alguien con reputación independiente real rinde 4.00; apadrinar a un títere (reputación independiente ~0) rinde 0.03. Las granjas padrino->títere no son rentables (§1.6).

## Conclusión
El prototipo confirma, en cifras, la apuesta central de la SPEC: **el poder estructural no se
compra con identidades ni con avales endogámicos**. Sybils y anillos de colusión quedan cerca
de 0% de poder de consenso; el damping anti-colusión es medible y necesario; el blanqueo no
compensa; y la responsabilidad persistente hace cara la colusión. El frente a seguir
endureciendo es §1.6 (colusión más sofisticada que un clique denso) — siguiente iteración.
