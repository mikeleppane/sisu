package main

import (
	"fmt"
	"math"
)

// A variable, so that each product rounds as in the other languages: a
// constant expression would be exact.
var pi = math.Pi
var solarMass = 4 * pi * pi

const daysPerYear = 365.24

type body struct {
	x, y, z, vx, vy, vz, mass float64
}

func planet(x, y, z, vx, vy, vz, mass float64) *body {
	return &body{x, y, z, vx * daysPerYear, vy * daysPerYear, vz * daysPerYear, mass * solarMass}
}

func offsetMomentum(bodies []*body) {
	var px, py, pz float64
	for _, b := range bodies {
		px += b.vx * b.mass
		py += b.vy * b.mass
		pz += b.vz * b.mass
	}
	bodies[0].vx = -px / solarMass
	bodies[0].vy = -py / solarMass
	bodies[0].vz = -pz / solarMass
}

func energy(bodies []*body) float64 {
	e := 0.0
	for i, a := range bodies {
		e += 0.5 * a.mass * (a.vx*a.vx + a.vy*a.vy + a.vz*a.vz)
		for _, b := range bodies[i+1:] {
			dx := a.x - b.x
			dy := a.y - b.y
			dz := a.z - b.z
			e -= a.mass * b.mass / math.Sqrt(dx*dx+dy*dy+dz*dz)
		}
	}
	return e
}

func advance(bodies []*body, dt float64) {
	for i, a := range bodies {
		for _, b := range bodies[i+1:] {
			dx := a.x - b.x
			dy := a.y - b.y
			dz := a.z - b.z
			d2 := dx*dx + dy*dy + dz*dz
			mag := dt / (d2 * math.Sqrt(d2))
			a.vx -= dx * b.mass * mag
			a.vy -= dy * b.mass * mag
			a.vz -= dz * b.mass * mag
			b.vx += dx * a.mass * mag
			b.vy += dy * a.mass * mag
			b.vz += dz * a.mass * mag
		}
	}
	for _, b := range bodies {
		b.x += dt * b.vx
		b.y += dt * b.vy
		b.z += dt * b.vz
	}
}

func main() {
	steps := 5000000
	bodies := []*body{
		{0, 0, 0, 0, 0, 0, solarMass},
		planet(4.84143144246472090, -1.16032004402742839, -1.03622044471123109e-1,
			1.66007664274403694e-3, 7.69901118419740425e-3, -6.90460016972063023e-5,
			9.54791938424326609e-4),
		planet(8.34336671824457987, 4.12479856412430479, -4.03523417114321381e-1,
			-2.76742510726862411e-3, 4.99852801234917238e-3, 2.30417297573763929e-5,
			2.85885980666130812e-4),
		planet(1.28943695621391310e1, -1.51111514016986312e1, -2.23307578892655734e-1,
			2.96460137564761618e-3, 2.37847173959480950e-3, -2.96589568540237556e-5,
			4.36624404335156298e-5),
		planet(1.53796971148509165e1, -2.59193146099879641e1, 1.79258772950371181e-1,
			2.68067772490389322e-3, 1.62824170038242295e-3, -9.51592254519715870e-5,
			5.15138902046611451e-5),
	}
	offsetMomentum(bodies)
	fmt.Printf("%.9f\n", energy(bodies))
	for i := 0; i < steps; i++ {
		advance(bodies, 0.01)
	}
	fmt.Printf("%.9f\n", energy(bodies))
}
