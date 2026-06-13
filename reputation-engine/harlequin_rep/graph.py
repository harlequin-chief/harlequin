"""
Grafo de confianza: las atestaciones (avales) entre seudónimos, por dimensión.

Ancla en SPEC.md:
- §1.3b: avalar = poner reputación en juego. Aquí el aval es una arista dirigida y ponderada.
- §1.6: ANTI-COLUSIÓN. La reputación que entra desde un anillo cerrado de votantes mutuos vale
  poco; la que viene de fuentes independientes y ya-reputadas vale más. Se implementa con
  `independencia(i, j)`: penaliza la reciprocidad (i<->j) y el solapamiento de vecinos (vivir en
  el mismo cluster). Un aval endogámico tiende a 0.
"""

from __future__ import annotations

from collections import Counter, defaultdict


class GrafoConfianza:
    """Aristas de atestación por dimensión: (origen -> destino, peso) dentro de una dimensión."""

    def __init__(self) -> None:
        # dim -> origen -> destino -> peso
        self._aristas: dict[str, dict[str, dict[str, float]]] = defaultdict(
            lambda: defaultdict(dict)
        )

    def atestar(self, origen: str, destino: str, dim: str, peso: float = 1.0) -> None:
        """origen avala a destino en la dimensión dim (§1.3b). Suma si ya existía."""
        if origen == destino:
            return  # nadie se avala a sí mismo
        actual = self._aristas[dim][origen].get(destino, 0.0)
        self._aristas[dim][origen][destino] = actual + peso

    def salientes(self, origen: str, dim: str) -> dict[str, float]:
        return dict(self._aristas[dim].get(origen, {}))

    def vecinos_salida(self, nodo: str, dim: str) -> set[str]:
        return set(self._aristas[dim].get(nodo, {}).keys())

    def tiene_arista(self, origen: str, destino: str, dim: str) -> bool:
        return destino in self._aristas[dim].get(origen, {})

    def independencia(
        self,
        i: str,
        j: str,
        dim: str,
        beta: float = 4.0,
        gamma: float = 4.0,
    ) -> float:
        """
        Factor de independencia del aval i->j en [0, 1] (§1.6).

        Penaliza dos firmas de colusión:
          - reciprocidad: si j también avala a i (anillo de avales mutuos).
          - solapamiento de vecinos (Jaccard de a quién avalan i y j): vivir en el mismo cluster
            cerrado -> avalan a la misma gente -> endogamia.

        independencia = 1 / (1 + beta*recíproco + gamma*solapamiento)
        Un aval entre dos miembros de un anillo cerrado (recíproco=1, solapamiento alto) cae a ~0.14
        o menos; un aval entre extraños bien conectados al resto de la red se queda cerca de 1.
        """
        reciproco = 1.0 if self.tiene_arista(j, i, dim) else 0.0

        ni = self.vecinos_salida(i, dim) - {j}
        nj = self.vecinos_salida(j, dim) - {i}
        union = ni | nj
        solapamiento = (len(ni & nj) / len(union)) if union else 0.0

        return 1.0 / (1.0 + beta * reciproco + gamma * solapamiento)

    def comunidades(self, dim: str, nodos: list[str]) -> dict[str, str]:
        """
        Detección de comunidades por **propagación de etiquetas** (label propagation) sobre la
        proyección no dirigida de los avales. Señal GLOBAL que un anillo de colusión deja aunque sea
        disperso (baja endogamia local): sigue siendo una comunidad densamente interconectada.
        Determinista (orden y desempates fijos) -> reproducible.
        """
        nodos_set = set(nodos)
        adj: dict[str, set[str]] = defaultdict(set)
        for i in nodos:
            for j in self.salientes(i, dim):
                if j in nodos_set:
                    adj[i].add(j)
                    adj[j].add(i)
        etiqueta = {n: n for n in nodos}
        orden = sorted(nodos)
        for _ in range(15):
            cambiado = False
            for n in orden:
                vecinos = adj.get(n)
                if not vecinos:
                    continue
                cuenta = Counter(etiqueta[m] for m in vecinos)
                mejor = max(sorted(cuenta), key=lambda k: cuenta[k])
                if etiqueta[n] != mejor:
                    etiqueta[n] = mejor
                    cambiado = True
            if not cambiado:
                break
        return etiqueta

    def sospecha_comunidades(
        self, dim: str, nodos: list[str], etiqueta: dict[str, str], evidencia: dict[str, float]
    ) -> dict[str, float]:
        """
        Sospecha por comunidad = aristas internas / (1 + evidencia de la comunidad). Alta cuando hay
        mucho aval mutuo y poca obra real -> firma de colusión, válida para anillos DENSOS y DISPERSOS
        (a diferencia de la independencia local, que solo ve cliques).
        """
        aristas_int: Counter = Counter()
        nodos_set = set(nodos)
        for i in nodos:
            for j in self.salientes(i, dim):
                if j in nodos_set and etiqueta[i] == etiqueta[j]:
                    aristas_int[etiqueta[i]] += 1
        ev: dict[str, float] = defaultdict(float)
        for n in nodos:
            ev[etiqueta[n]] += evidencia.get(n, 0.0)
        return {
            comm: aristas_int.get(comm, 0) / (1.0 + ev.get(comm, 0.0))
            for comm in set(etiqueta.values())
        }

    def matriz_local_amortiguada(
        self,
        dim: str,
        nodos: list[str],
        damping: bool = True,
        comunidad: bool = False,
        evidencia: dict[str, float] | None = None,
        kappa: float = 0.5,
    ) -> dict[str, dict[str, float]]:
        """
        Matriz de confianza local C, fila-estocástica, con damping anti-colusión aplicado (§1.6).

        C[i][j] = peso(i->j) * independencia(i, j), normalizado para que la fila i sume 1.
        Si i no avala a nadie (fila vacía), se deja vacía -> EigenTrust lo trata como "colgante"
        y reparte su masa hacia el pre-trust (anclaje en evidencia real).

        `damping=False` desactiva el factor de independencia (sirve para MEDIR cuánto aporta el
        anti-colusión: comparar reputación con vs sin damping ante un anillo de colusión).

        CLAVE (corrección de diseño): la normalización se hace por la suma de pesos SIN amortiguar.
        Así, cuando un nodo solo avala dentro de un anillo endogámico (independencia baja en TODAS
        sus aristas), su fila suma << 1 -> es SUB-estocástica -> la mayor parte de su confianza
        "se fuga" en vez de propagarse al anillo. Si en cambio normalizásemos por la suma ya
        amortiguada, un factor uniforme se cancelaría y el damping no haría nada. El déficit de fila
        (1 - suma) lo reinyecta el cálculo de reputación hacia el pre-trust (anclaje en evidencia).
        """
        nodos_set = set(nodos)

        # damping por comunidad (opt-in): factor global que castiga aristas dentro de una comunidad
        # sospechosa (muchas aristas internas, poca evidencia). Cierra el hueco del anillo disperso.
        etiqueta: dict[str, str] = {}
        sospecha: dict[str, float] = {}
        if damping and comunidad and evidencia is not None:
            etiqueta = self.comunidades(dim, nodos)
            sospecha = self.sospecha_comunidades(dim, nodos, etiqueta, evidencia)

        def factor(i: str, j: str) -> float:
            if not damping:
                return 1.0
            f = self.independencia(i, j, dim)
            if etiqueta and etiqueta.get(i) == etiqueta.get(j):
                f *= 1.0 / (1.0 + kappa * sospecha.get(etiqueta[i], 0.0))
            return f

        C: dict[str, dict[str, float]] = {}
        for i in nodos:
            salientes = {j: p for j, p in self.salientes(i, dim).items() if j in nodos_set}
            suma_bruta = sum(salientes.values())
            if suma_bruta <= 0:
                C[i] = {}
                continue
            C[i] = {j: peso * factor(i, j) / suma_bruta for j, peso in salientes.items()}
        return C
