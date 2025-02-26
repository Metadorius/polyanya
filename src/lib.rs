use std::{
    cmp::Ordering,
    collections::BinaryHeap,
    fmt::{self, Display},
    hash::Hash,
    io,
    io::BufRead,
};

use hashbrown::{hash_map::Entry, HashMap};
use helpers::{distance_between, heuristic, on_side};
#[cfg(feature = "tracing")]
use tracing::instrument;

use crate::helpers::{line_intersect_segment, on_segment, turning_on};

mod helpers;

#[derive(Debug)]
pub struct Vertex {
    x: f32,
    y: f32,
    polygons: Vec<isize>,
    is_corner: bool,
}

impl Vertex {
    pub fn new(x: u32, y: u32, poly: Vec<isize>) -> Self {
        Vertex {
            x: x as f32,
            y: y as f32,
            is_corner: poly.contains(&-1),
            polygons: poly,
        }
    }

    #[inline(always)]
    fn p(&self) -> [f32; 2] {
        [self.x, self.y]
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum SuccessorType {
    LeftNonObservable,
    Observable,
    RightNonObservable,
}

#[derive(Debug, PartialEq, Clone, Copy)]
struct Successor {
    interval: [[f32; 2]; 2],
    edge: [usize; 2],
    ty: SuccessorType,
}

#[derive(Debug, PartialEq)]
pub struct Path {
    pub len: f32,
    pub path: Vec<[f32; 2]>,
}

#[derive(Debug)]
pub struct Polygon {
    vertices: Vec<usize>,
    // neighbours: Vec<isize>,
    is_one_way: bool,
}

impl Polygon {
    pub fn new(nb: usize, data: Vec<isize>) -> Self {
        assert!(data.len() == nb * 2);
        let (vertices, neighbours) = data.split_at(nb);
        let vertices = vertices.iter().copied().map(|v| v as usize).collect();
        let neighbours = neighbours.to_vec();
        let mut found_trav = false;
        let mut is_one_way = true;
        for neighbour in &neighbours {
            if *neighbour != -1 {
                if found_trav {
                    is_one_way = false;
                    break;
                } else {
                    found_trav = true;
                }
            }
        }
        Polygon {
            vertices,
            // neighbours,
            is_one_way,
        }
    }

    #[cfg_attr(feature = "tracing", instrument(skip_all))]
    #[inline(always)]
    fn edges_index(&self) -> Vec<[usize; 2]> {
        let mut edges = Vec::with_capacity(self.vertices.len() / 2);
        let mut last = self.vertices[0];
        for vertex in self.vertices.iter().skip(1) {
            edges.push([last, *vertex]);
            last = *vertex;
        }
        edges.push([last, self.vertices[0]]);
        edges
    }

    #[cfg_attr(feature = "tracing", instrument(skip_all))]
    #[inline(always)]
    fn double_edges_index(&self) -> Vec<[usize; 2]> {
        let mut edges = Vec::with_capacity(self.vertices.len());
        let mut last = self.vertices[0];
        for vertex in self.vertices.iter().skip(1) {
            edges.push([last, *vertex]);
            last = *vertex;
        }
        edges.push([last, self.vertices[0]]);
        let mut last = self.vertices[0];
        for vertex in self.vertices.iter().skip(1) {
            edges.push([last, *vertex]);
            last = *vertex;
        }
        edges.push([last, self.vertices[0]]);
        edges
    }
}

#[derive(Debug, Default)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub polygons: Vec<Polygon>,
}

struct Root([f32; 2]);
impl PartialEq for Root {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for Root {}
impl Hash for Root {
    #[inline(always)]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        ((self.0[0] * 10000.0) as i32).hash(state);
        ((self.0[1] * 10000.0) as i32).hash(state);
        state.finish();
    }
}

impl Mesh {
    pub fn from_file(path: &str) -> Mesh {
        let file = std::fs::File::open(path).unwrap();
        let mut mesh = Mesh::default();
        let mut nb_vertices = 0;
        let mut nb_polygons = 0;
        let mut phase = 0;
        for line in io::BufReader::new(file).lines() {
            let line: String = line.unwrap();
            if phase == 0 {
                if line == "mesh" || line == "2" {
                    continue;
                } else {
                    (nb_vertices, nb_polygons) = line
                        .split_once(' ')
                        .map(|(a, b)| (a.parse().unwrap(), b.parse().unwrap()))
                        .unwrap();
                    phase = 1;
                    continue;
                }
            }
            if phase == 1 {
                if nb_vertices > 0 {
                    nb_vertices -= 1;
                    let mut values = line.split(' ');
                    let x = values.next().unwrap().parse().unwrap();
                    let y = values.next().unwrap().parse().unwrap();
                    let _ = values.next();
                    let vertex = Vertex::new(x, y, values.map(|v| v.parse().unwrap()).collect());
                    mesh.vertices.push(vertex);
                } else {
                    phase = 2;
                }
            }
            if phase == 2 {
                if nb_polygons > 0 {
                    nb_polygons -= 1;
                    let mut values = line.split(' ');
                    let n = values.next().unwrap().parse().unwrap();
                    let polygon = Polygon::new(n, values.map(|v| v.parse().unwrap()).collect());
                    mesh.polygons.push(polygon)
                } else {
                    panic!("unexpected line");
                }
            }
        }
        mesh
    }
}

struct SearchInstance<'m> {
    queue: BinaryHeap<SearchNode>,
    node_buffer: Vec<SearchNode>,
    root_history: HashMap<Root, f32>,
    to: [f32; 2],
    polygon_to: isize,
    mesh: &'m Mesh,
    #[cfg(feature = "stats")]
    pushed: usize,
    #[cfg(feature = "stats")]
    popped: usize,
    #[cfg(feature = "stats")]
    successors_called: usize,
    #[cfg(feature = "stats")]
    nodes_generated: usize,
    #[cfg(debug_assertions)]
    debug: bool,
    #[cfg(debug_assertions)]
    fail_fast: i32,
}

impl Mesh {
    #[cfg_attr(feature = "tracing", instrument(skip_all))]
    pub fn path(&self, from: [f32; 2], to: [f32; 2]) -> Path {
        let starting_polygon_index = self.point_in_polygon(from);
        let starting_polygon = self.polygons.get(starting_polygon_index).unwrap();
        let ending_polygon = self.point_in_polygon(to);

        if starting_polygon_index == ending_polygon {
            return Path {
                len: distance_between(from, to),
                path: vec![to],
            };
        }

        let mut search_instance = SearchInstance {
            queue: BinaryHeap::with_capacity(15),
            node_buffer: Vec::with_capacity(10),
            root_history: HashMap::with_capacity(10),
            to,
            polygon_to: ending_polygon as isize,
            mesh: self,
            #[cfg(feature = "stats")]
            pushed: 0,
            #[cfg(feature = "stats")]
            popped: 0,
            #[cfg(feature = "stats")]
            successors_called: 0,
            #[cfg(feature = "stats")]
            nodes_generated: 0,
            #[cfg(debug_assertions)]
            debug: false,
            #[cfg(debug_assertions)]
            fail_fast: -1,
        };
        search_instance.root_history.insert(Root(from), 0.0);

        let empty_node = SearchNode {
            path: vec![],
            r: from,
            i: [[0.0, 0.0], [0.0, 0.0]],
            i_index: [0, 0],
            polygon_from: -1,
            polygon_to: starting_polygon_index as isize,
            f: 0.0,
            g: 0.0,
        };

        for edge in starting_polygon.edges_index() {
            let start = self.vertices.get(edge[0]).unwrap();
            let end = self.vertices.get(edge[1]).unwrap();

            let mut other_side = isize::MAX;
            for i in &start.polygons {
                if *i != -1 && *i != starting_polygon_index as isize && end.polygons.contains(i) {
                    other_side = *i;
                }
            }

            search_instance.add_node(
                from,
                other_side,
                ([start.x, start.y], edge[0]),
                ([end.x, end.y], edge[1]),
                &empty_node,
            );
        }
        search_instance.flush_nodes();

        while let Some(next) = search_instance.queue.pop() {
            #[cfg(feature = "verbose")]
            println!("popped off: {}", next);
            #[cfg(feature = "stats")]
            {
                search_instance.popped += 1;
            }
            if next.polygon_to == ending_polygon as isize {
                #[cfg(feature = "stats")]
                eprintln!(
                    "{:?} / {:?} / {:?} / {:?}",
                    search_instance.successors_called,
                    search_instance.nodes_generated,
                    search_instance.pushed,
                    search_instance.popped
                );
                let mut path = next
                    .path
                    .split_first()
                    .map(|(_, p)| p)
                    .unwrap_or(&[])
                    .to_vec();
                if next.r != from {
                    path.push(next.r);
                }
                if let Some(turn) = turning_on(next.r, to, next.i) {
                    path.push(turn);
                }
                path.push(to);
                return Path {
                    path,
                    len: next.f + next.g,
                };
            }
            search_instance.successors(next);
        }
        Path {
            path: vec![],
            len: -1.0,
        }
    }

    #[cfg_attr(feature = "tracing", instrument(skip_all))]
    #[cfg(test)]
    fn successors(&self, node: SearchNode, to: [f32; 2]) -> Vec<SearchNode> {
        let mut search_instance = SearchInstance {
            queue: BinaryHeap::new(),
            node_buffer: Vec::new(),
            root_history: HashMap::new(),
            to,
            polygon_to: self.point_in_polygon(to) as isize,
            mesh: self,
            #[cfg(feature = "stats")]
            pushed: 0,
            #[cfg(feature = "stats")]
            popped: 0,
            #[cfg(feature = "stats")]
            successors_called: 0,
            #[cfg(feature = "stats")]
            nodes_generated: 0,
            #[cfg(debug_assertions)]
            debug: false,
            #[cfg(debug_assertions)]
            fail_fast: -1,
        };
        search_instance.successors(node);
        search_instance.queue.drain().collect()
    }
    #[cfg_attr(feature = "tracing", instrument(skip_all))]
    #[cfg(test)]
    fn edges_between(&self, node: &SearchNode) -> Vec<Successor> {
        let search_instance = SearchInstance {
            queue: BinaryHeap::new(),
            node_buffer: Vec::new(),
            root_history: HashMap::new(),
            to: [0.0, 0.0],
            polygon_to: self.point_in_polygon([0.0, 0.0]) as isize,
            mesh: self,
            #[cfg(feature = "stats")]
            pushed: 0,
            #[cfg(feature = "stats")]
            popped: 0,
            #[cfg(feature = "stats")]
            successors_called: 0,
            #[cfg(feature = "stats")]
            nodes_generated: 0,
            #[cfg(debug_assertions)]
            debug: false,
            #[cfg(debug_assertions)]
            fail_fast: -1,
        };
        search_instance.edges_between(node)
    }
}

impl<'m> SearchInstance<'m> {
    #[cfg_attr(feature = "tracing", instrument(skip_all))]
    #[inline(always)]
    fn edges_between(&self, node: &SearchNode) -> Vec<Successor> {
        let mut successors = vec![];

        let polygon = self.mesh.polygons.get(node.polygon_to as usize).unwrap();

        if distance_between(node.i[0], node.r) < 1.0e-5
            || distance_between(node.i[1], node.r) < 1.0e-5
            || on_side(node.r, node.i) == EdgeSide::Edge
        {
            // println!("collinear");
            // TODO: possible optimisation
        }
        if polygon.vertices.len() == 3 {
            // println!("triangle");
            // TODO: possible optimisation
        }

        let right_index = {
            let mut temp = 0;
            while polygon.vertices[temp] != node.i_index[1] {
                temp += 1;
            }
            temp + 1
        };
        let left_index = polygon.vertices.len() + right_index - 1 - 1;

        let mut ty = SuccessorType::RightNonObservable;
        for edge in &polygon.double_edges_index()[right_index..=left_index] {
            let start = self.mesh.vertices.get(edge[0]).unwrap();
            let end = self.mesh.vertices.get(edge[1]).unwrap();
            let mut start_p = start.p();

            #[cfg(debug_assertions)]
            if self.debug {
                println!("| {:?} : {:?} / {:?}", edge, start_p, end.p());
                println!(
                    "|   {:?} - {:?}",
                    on_side([start.x, start.y], [node.r, node.i[0]]),
                    on_side([start.x, start.y], [node.r, node.i[1]])
                );
                println!(
                    "|   {:?} - {:?}",
                    on_side([end.x, end.y], [node.r, node.i[0]]),
                    on_side([end.x, end.y], [node.r, node.i[1]])
                );
            }

            match on_side([start.x, start.y], [node.r, node.i[0]]) {
                EdgeSide::Right => {
                    if let Some(intersect) = line_intersect_segment(
                        [node.r, node.i[0]],
                        [[start.x, start.y], [end.x, end.y]],
                    ) {
                        #[cfg(debug_assertions)]
                        if self.debug {
                            println!("|   intersection 0 {:?}", intersect);
                            println!(
                                "|     {:?} / {:?}",
                                distance_between(intersect, start_p),
                                distance_between(intersect, end.p())
                            );
                        }
                        if distance_between(intersect, start_p) > 1.0e-3
                            && distance_between(intersect, end.p()) > 1.0e-3
                        {
                            successors.push(Successor {
                                interval: [start_p, intersect],
                                edge: *edge,
                                ty,
                            });
                            start_p = intersect;
                        } else {
                            #[cfg(debug_assertions)]
                            if self.debug {
                                println!("|     ignoring intersection");
                            }
                        }
                        if distance_between(intersect, end.p()) > 1.0e-3 {
                            ty = SuccessorType::Observable;
                        }
                    }
                }
                EdgeSide::Left => {
                    if ty == SuccessorType::RightNonObservable {
                        ty = SuccessorType::Observable;
                    }
                }
                EdgeSide::Edge => match on_side([end.x, end.y], [node.r, node.i[0]]) {
                    EdgeSide::Edge | EdgeSide::Left => {
                        ty = SuccessorType::Observable;
                    }
                    _ => (),
                },
            }
            let mut end_intersection_p = None;
            let mut found_intersection = false;
            if on_side([end.x, end.y], [node.r, node.i[1]]) == EdgeSide::Left {
                if let Some(intersect) = line_intersect_segment(
                    [node.r, node.i[1]],
                    [[start.x, start.y], [end.x, end.y]],
                ) {
                    #[cfg(debug_assertions)]
                    if self.debug {
                        println!("|   intersection 1 {:?}", intersect);
                        println!(
                            "|     {:?} / {:?}",
                            distance_between(intersect, start_p),
                            distance_between(intersect, end.p())
                        );
                    }

                    if distance_between(intersect, end.p()) > 1.0e-3 {
                        end_intersection_p = Some(intersect);
                    } else {
                        #[cfg(debug_assertions)]
                        if self.debug {
                            println!("|     ignoring intersection");
                        }
                    }
                    found_intersection = true;
                }
            }
            successors.push(Successor {
                interval: [start_p, end_intersection_p.unwrap_or_else(|| end.p())],
                edge: *edge,
                ty,
            });
            match on_side([end.x, end.y], [node.r, node.i[1]]) {
                EdgeSide::Left => {
                    if found_intersection {
                        ty = SuccessorType::LeftNonObservable;
                    }
                    if let Some(intersect) = end_intersection_p {
                        successors.push(Successor {
                            interval: [intersect, end.p()],
                            edge: *edge,
                            ty,
                        });
                    }
                }
                EdgeSide::Edge => match on_side([end.x, end.y], [node.r, node.i[0]]) {
                    EdgeSide::Edge | EdgeSide::Left => {
                        ty = SuccessorType::LeftNonObservable;
                    }
                    _ => (),
                },
                _ => (),
            }
        }

        successors
    }

    #[cfg_attr(feature = "tracing", instrument(skip_all))]
    #[inline(always)]
    fn add_node(
        &mut self,
        root: [f32; 2],
        other_side: isize,
        start: ([f32; 2], usize),
        end: ([f32; 2], usize),
        node: &SearchNode,
    ) {
        #[cfg(feature = "stats")]
        {
            self.nodes_generated += 1;
        }
        // prune edges that don't have a polygon on the other side: cul de sac pruning
        if other_side == isize::MAX {
            #[cfg(debug_assertions)]
            if self.debug {
                println!("x cul de sac");
            }

            return;
        }

        // prune edges that only lead to one other polygon, and not the target: dead end pruning
        if self.polygon_to != other_side
            && self
                .mesh
                .polygons
                .get(other_side as usize)
                .unwrap()
                .is_one_way
        {
            #[cfg(debug_assertions)]
            if self.debug {
                println!("x dead end");
            }

            return;
        }

        let mut path = node.path.clone();
        if root != node.r {
            path.push(node.r);
        }

        let heuristic = heuristic(root, self.to, [start.0, end.0]);
        let new_node = SearchNode {
            path,
            r: root,
            i: [start.0, end.0],
            i_index: [start.1, end.1],
            polygon_from: node.polygon_to as isize,
            polygon_to: other_side,
            f: node.f + distance_between(node.r, root),
            g: heuristic,
        };
        if new_node.f.is_nan() || new_node.g.is_nan() {
            #[cfg(debug_assertions)]
            if self.debug {
                println!("x one of the distance is NaN");
            }

            return;
        }

        match self.root_history.entry(Root(root)) {
            Entry::Occupied(mut o) => {
                if o.get() < &new_node.f {
                    #[cfg(debug_assertions)]
                    if self.debug {
                        println!("x already got a better path");
                    }
                } else {
                    #[cfg(debug_assertions)]
                    if self.debug {
                        println!("o added!");
                    }
                    o.insert(new_node.f);
                    self.node_buffer.push(new_node);
                }
            }
            Entry::Vacant(v) => {
                #[cfg(debug_assertions)]
                if self.debug {
                    println!("o added!");
                }
                v.insert(new_node.f);
                self.node_buffer.push(new_node);
            }
        }
    }

    #[cfg_attr(feature = "tracing", instrument(skip_all))]
    #[inline(always)]
    fn flush_nodes(&mut self) {
        #[cfg(feature = "stats")]
        {
            self.pushed += self.node_buffer.len();
        }
        #[cfg(feature = "verbose")]
        for new_node in &self.node_buffer {
            println!("        pushing: {}", new_node);
        }
        self.queue.extend(self.node_buffer.drain(..));
    }

    #[cfg_attr(feature = "tracing", instrument(skip_all))]
    #[inline(always)]
    fn successors(&mut self, mut node: SearchNode) {
        #[cfg(feature = "stats")]
        {
            self.successors_called += 1;
        }
        loop {
            #[cfg(debug_assertions)]
            // select a search node to enable debug more
            if false {
                self.debug = true;
                self.fail_fast = 3;
            }
            for successor in self.edges_between(&node) {
                let start = self.mesh.vertices.get(successor.edge[0]).unwrap();
                let end = self.mesh.vertices.get(successor.edge[1]).unwrap();

                #[cfg(debug_assertions)]
                if self.debug {
                    println!("v {:?}", successor);
                }

                let mut other_side = isize::MAX;
                // find the polygon at the other side of this edge
                for i in &start.polygons {
                    if *i != -1 && *i != node.polygon_to as isize && end.polygons.contains(i) {
                        other_side = *i;
                    }
                }

                #[cfg(debug_assertions)]
                if self.debug {
                    println!("| going to {:?}", other_side);
                }

                let root = match successor.ty {
                    SuccessorType::RightNonObservable => {
                        if distance_between(successor.interval[0], start.p()) > 1.0e-5 {
                            #[cfg(debug_assertions)]
                            if self.debug {
                                println!("x non observable on an intersection");
                            }
                            continue;
                        }
                        let vertex = self.mesh.vertices.get(node.i_index[0]).unwrap();
                        if vertex.is_corner && distance_between(vertex.p(), node.i[0]) < 1.0e-5 {
                            node.i[0]
                        } else {
                            #[cfg(debug_assertions)]
                            if self.debug {
                                println!("x non observable on an non corner");
                            }
                            continue;
                        }
                    }
                    SuccessorType::Observable => node.r,
                    SuccessorType::LeftNonObservable => {
                        if distance_between(successor.interval[1], end.p()) > 1.0e-5 {
                            #[cfg(debug_assertions)]
                            if self.debug {
                                println!("x non observable on an intersection");
                            }
                            continue;
                        }
                        let vertex = self.mesh.vertices.get(node.i_index[1]).unwrap();
                        if vertex.is_corner && distance_between(vertex.p(), node.i[1]) < 1.0e-5 {
                            node.i[1]
                        } else {
                            #[cfg(debug_assertions)]
                            if self.debug {
                                println!("x non observable on an non corner");
                            }
                            continue;
                        }
                    }
                };

                #[cfg(debug_assertions)]
                if self.debug {
                    println!("| through root {:?}", root);
                }

                self.add_node(
                    root,
                    other_side,
                    (successor.interval[0], successor.edge[0]),
                    (successor.interval[1], successor.edge[1]),
                    &node,
                )
            }

            if self.node_buffer.len() == 1 && self.node_buffer[0].polygon_to != self.polygon_to {
                #[cfg(feature = "verbose")]
                for new_node in &self.node_buffer {
                    println!("        intermediate: {}", new_node);
                }
                node = self.node_buffer.drain(..).next().unwrap();
                #[cfg(debug_assertions)]
                {
                    self.fail_fast -= 1;
                    if self.fail_fast == 0 {
                        panic!()
                    }
                }
            } else {
                #[cfg(debug_assertions)]
                {
                    self.fail_fast -= 1;
                    if self.fail_fast == 0 {
                        panic!()
                    }
                }
                break;
            }
        }
        self.flush_nodes();
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum EdgeSide {
    Left,
    Right,
    Edge,
}

impl Mesh {
    pub fn point_in_mesh(&self, point: [f32; 2]) -> bool {
        self.point_in_polygon(point) != usize::MAX
    }

    #[cfg_attr(feature = "tracing", instrument(skip_all))]
    fn point_in_polygon(&self, point: [f32; 2]) -> usize {
        let delta = 0.1;
        [
            [0.0, 0.0],
            [delta, 0.0],
            [delta, delta],
            [0.0, delta],
            [-delta, delta],
            [-delta, 0.0],
            [-delta, -delta],
            [0.0, -delta],
            [delta, -delta],
        ]
        .iter()
        .map(|delta| self.point_in_polygon_unit([point[0] + delta[0], point[1] + delta[1]]))
        .find(|poly| *poly != usize::MAX)
        .unwrap_or(usize::MAX)
    }

    #[cfg_attr(feature = "tracing", instrument(skip_all))]
    fn point_in_polygon_unit(&self, point: [f32; 2]) -> usize {
        'polygons: for (i, polygon) in self.polygons.iter().enumerate() {
            for edge in polygon.edges_index() {
                let last = self.vertices.get(edge[0]).unwrap();
                let next = self.vertices.get(edge[1]).unwrap();
                let current_side = on_side(point, [[last.x, last.y], [next.x, next.y]]);
                if on_segment(point, [[last.x, last.y], [next.x, next.y]]) {
                    return i;
                }
                if current_side != EdgeSide::Left {
                    continue 'polygons;
                }
            }
            return i;
        }
        usize::MAX
    }
}

#[derive(PartialEq, Debug)]
struct SearchNode {
    path: Vec<[f32; 2]>,
    r: [f32; 2],
    i: [[f32; 2]; 2],
    i_index: [usize; 2],
    polygon_from: isize,
    polygon_to: isize,
    f: f32,
    g: f32,
}

impl Display for SearchNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&format!("root=({}, {}); ", self.r[0], self.r[1]))?;
        f.write_str(&format!("left=({}, {}); ", self.i[1][0], self.i[1][1]))?;
        f.write_str(&format!("right=({}, {}); ", self.i[0][0], self.i[0][1]))?;
        f.write_str(&format!("f={:.2}, g={:.2} ", self.f + self.g, self.f))?;
        Ok(())
    }
}

impl PartialOrd for SearchNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for SearchNode {}

impl Ord for SearchNode {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.f + self.g).total_cmp(&(other.f + other.g)) {
            Ordering::Less => Ordering::Greater,
            Ordering::Equal => Ordering::Equal,
            Ordering::Greater => Ordering::Less,
        }
    }
}

#[cfg(test)]
mod tests {
    macro_rules! assert_delta {
        ($x:expr, $y:expr) => {
            let val = $x;
            let expected = $y;
            if !((val - expected).abs() < 0.01) {
                assert_eq!(val, expected);
            }
        };
    }

    use crate::{
        helpers::{distance_between, mirror},
        Mesh, Path, Polygon, SearchNode, Vertex,
    };

    fn mesh_u_grid() -> Mesh {
        Mesh {
            vertices: vec![
                Vertex::new(0, 0, vec![0, -1]),
                Vertex::new(1, 0, vec![0, 1, -1]),
                Vertex::new(2, 0, vec![1, 2, -1]),
                Vertex::new(3, 0, vec![2, -1]),
                Vertex::new(0, 1, vec![3, 0, -1]),
                Vertex::new(1, 1, vec![3, 1, 0, -1]),
                Vertex::new(2, 1, vec![4, 2, 1, -1]),
                Vertex::new(3, 1, vec![4, 2, -1]),
                Vertex::new(0, 2, vec![3, -1]),
                Vertex::new(1, 2, vec![3, -1]),
                Vertex::new(2, 2, vec![4, -1]),
                Vertex::new(3, 2, vec![4, -1]),
            ],
            polygons: vec![
                Polygon::new(4, vec![0, 1, 5, 4, -1, 1, 3, -1]),
                Polygon::new(4, vec![1, 2, 6, 5, -1, 2, -1, 0]),
                Polygon::new(4, vec![2, 3, 7, 6, -1, -1, 4, 1]),
                Polygon::new(4, vec![4, 5, 9, 8, 0, -1, -1, -1]),
                Polygon::new(4, vec![6, 7, 11, 10, 2, -1, -1, -1]),
            ],
        }
    }

    #[test]
    fn point_in_polygon() {
        let mesh = mesh_u_grid();
        assert_eq!(mesh.point_in_polygon([0.5, 0.5]), 0);
        assert_eq!(mesh.point_in_polygon([1.5, 0.5]), 1);
        assert_eq!(mesh.point_in_polygon([0.5, 1.5]), 3);
        assert_eq!(mesh.point_in_polygon([1.5, 1.5]), usize::MAX);
        assert_eq!(mesh.point_in_polygon([2.5, 1.5]), 4);
    }

    #[test]
    fn successors_straight_line_ahead() {
        let mesh = mesh_u_grid();

        let from = [0.1, 0.1];
        let to = [2.9, 0.9];
        let search_node = SearchNode {
            path: vec![],
            r: from,
            i: [[1.0, 0.0], [1.0, 1.0]],
            i_index: [1, 5],
            polygon_from: mesh.point_in_polygon(from) as isize,
            polygon_to: 1,
            f: 0.0,
            g: distance_between(from, to),
        };
        let successors = dbg!(mesh.successors(search_node, to));
        assert_eq!(successors.len(), 1);
        assert_eq!(successors[0].r, from);
        assert_eq!(successors[0].f, 0.0);
        assert_eq!(successors[0].g, distance_between(from, to));
        assert_eq!(successors[0].polygon_from, 1);
        assert_eq!(successors[0].polygon_to, 2);
        assert_eq!(successors[0].i, [[2.0, 0.0], [2.0, 1.0]]);
        assert_eq!(successors[0].i_index, [2, 6]);

        assert_eq!(successors[0].path, Vec::<[f32; 2]>::new());

        assert_eq!(
            mesh.path(from, to),
            Path {
                path: vec![to],
                len: distance_between(from, to)
            }
        );
    }

    #[test]
    fn successors_straight_line_reversed() {
        let mesh = mesh_u_grid();

        let to = [0.1, 0.1];
        let from = [2.9, 0.9];
        let search_node = SearchNode {
            path: vec![],
            r: from,
            i: [[2.0, 1.0], [2.0, 0.0]],
            i_index: [6, 2],
            polygon_from: mesh.point_in_polygon(from) as isize,
            polygon_to: 1,
            f: 0.0,
            g: distance_between(from, to),
        };
        let successors = mesh.successors(search_node, to);
        assert_eq!(successors.len(), 1);
        assert_eq!(successors[0].r, from);
        assert_eq!(successors[0].f, 0.0);
        assert_eq!(successors[0].g, distance_between(to, from));
        assert_eq!(successors[0].polygon_from, 1);
        assert_eq!(successors[0].polygon_to, 0);
        assert_eq!(successors[0].i, [[1.0, 1.0], [1.0, 0.0]]);
        assert_eq!(successors[0].i_index, [5, 1]);
        assert_eq!(successors[0].path, Vec::<[f32; 2]>::new());

        assert_eq!(
            mesh.path(from, to),
            Path {
                path: vec![to],
                len: distance_between(from, to)
            }
        );
    }

    #[test]
    fn successors_corner_first_step() {
        let mesh = mesh_u_grid();

        let from = [0.1, 1.9];
        let to = [2.1, 1.9];
        let search_node = SearchNode {
            path: vec![],
            r: from,
            i: [[0.0, 1.0], [1.0, 1.0]],
            i_index: [4, 5],
            polygon_from: mesh.point_in_polygon(from) as isize,
            polygon_to: 0,
            f: 0.0,
            g: distance_between(from, to),
        };
        let successors = dbg!(mesh.successors(search_node, to));
        assert_eq!(successors.len(), 1);
        assert_eq!(successors[0].r, [2.0, 1.0]);
        assert_eq!(
            successors[0].f,
            distance_between(from, [1.0, 1.0]) + distance_between([1.0, 1.0], [2.0, 1.0])
        );
        assert_eq!(successors[0].g, distance_between([2.0, 1.0], to));
        assert_eq!(successors[0].polygon_from, 2);
        assert_eq!(successors[0].polygon_to, 4);
        assert_eq!(successors[0].i, [[3.0, 1.0], [2.0, 1.0]]);
        assert_eq!(successors[0].i_index, [7, 6]);
        assert_eq!(successors[0].path, vec![from, [1.0, 1.0]]);

        assert_eq!(
            mesh.path(from, to),
            Path {
                path: vec![[1.0, 1.0], [2.0, 1.0], to],
                len: distance_between(from, [1.0, 1.0])
                    + distance_between([1.0, 1.0], [2.0, 1.0])
                    + distance_between([2.0, 1.0], to)
            }
        );
    }

    #[test]
    fn successors_corner_observable_second_step() {
        let mesh = mesh_u_grid();

        let from = [0.1, 1.9];
        let to = [2.1, 1.9];
        let search_node = SearchNode {
            path: vec![],
            r: from,
            i: [[1.0, 0.0], [1.0, 1.0]],
            i_index: [1, 5],

            polygon_from: 0,
            polygon_to: 1,
            f: 0.0,
            g: distance_between(from, to),
        };
        let successors = dbg!(mesh.successors(search_node, to));
        assert_eq!(successors.len(), 1);
        assert_eq!(successors[0].r, [2.0, 1.0]);
        assert_eq!(
            successors[0].f,
            distance_between(from, [1.0, 1.0]) + distance_between([1.0, 1.0], [2.0, 1.0])
        );
        assert_eq!(successors[0].g, distance_between([2.0, 1.0], to));
        assert_eq!(successors[0].polygon_from, 2);
        assert_eq!(successors[0].polygon_to, 4);
        assert_eq!(successors[0].i, [[3.0, 1.0], [2.0, 1.0]]);
        assert_eq!(successors[0].i_index, [7, 6]);
        assert_eq!(successors[0].path, vec![from, [1.0, 1.0]]);

        assert_eq!(
            mesh.path(from, to),
            Path {
                path: vec![[1.0, 1.0], [2.0, 1.0], to],
                len: distance_between(from, [1.0, 1.0])
                    + distance_between([1.0, 1.0], [2.0, 1.0])
                    + distance_between([2.0, 1.0], to)
            }
        );
    }

    fn mesh_from_paper() -> Mesh {
        Mesh {
            vertices: vec![
                Vertex::new(0, 6, vec![0, -1]),           // 0
                Vertex::new(2, 5, vec![0, -1, 2]),        // 1
                Vertex::new(5, 7, vec![0, 2, -1]),        // 2
                Vertex::new(5, 8, vec![0, -1]),           // 3
                Vertex::new(0, 8, vec![0, -1]),           // 4
                Vertex::new(1, 4, vec![1, -1]),           // 5
                Vertex::new(2, 1, vec![1, -1]),           // 6
                Vertex::new(4, 1, vec![1, -1]),           // 7
                Vertex::new(4, 2, vec![1, -1, 2]),        // 8
                Vertex::new(2, 4, vec![1, 2, -1]),        // 9
                Vertex::new(7, 4, vec![2, -1, 4]),        // 10
                Vertex::new(10, 7, vec![2, 4, 6, -1, 3]), // 11
                Vertex::new(7, 7, vec![2, 3, -1]),        // 12
                Vertex::new(11, 8, vec![3, -1]),          // 13
                Vertex::new(7, 8, vec![3, -1]),           // 14
                Vertex::new(7, 0, vec![5, 4, -1]),        // 15
                Vertex::new(11, 3, vec![4, 5, -1]),       // 16
                Vertex::new(11, 5, vec![4, -1, 6]),       // 17
                Vertex::new(12, 0, vec![5, -1]),          // 18
                Vertex::new(12, 3, vec![5, -1]),          // 19
                Vertex::new(13, 5, vec![6, -1]),          // 20
                Vertex::new(13, 7, vec![6, -1]),          // 21
                Vertex::new(1, 3, vec![1, -1]),           // 22
            ],
            polygons: vec![
                Polygon::new(5, vec![0, 1, 2, 3, 4, -1, -1, 2, -1, -1]),
                Polygon::new(6, vec![5, 22, 6, 7, 8, 9, -1, -1, -1, -1, 2, -1]),
                Polygon::new(7, vec![1, 9, 8, 10, 11, 12, 2, -1, 1, -1, 4, 3, -1, 0]),
                Polygon::new(4, vec![12, 11, 13, 14, 2, -1, -1, -1]),
                Polygon::new(5, vec![10, 15, 16, 17, 11, -1, 5, -1, 6, 2]),
                Polygon::new(4, vec![15, 18, 19, 16, -1, -1, -1, 4]),
                Polygon::new(4, vec![11, 17, 20, 21, 4, -1, -1, -1]),
            ],
        }
    }

    #[test]
    fn paper_straight() {
        let mesh = mesh_from_paper();

        let from = [12.0, 0.0];
        let to = [7.0, 6.9];
        let search_node = SearchNode {
            path: vec![],
            r: from,
            i: [[11.0, 3.0], [7.0, 0.0]],
            i_index: [16, 15],
            polygon_from: mesh.point_in_polygon(from) as isize,
            polygon_to: 4,
            f: 0.0,
            g: distance_between(from, to),
        };
        let successors = dbg!(mesh.successors(search_node, to));
        assert_eq!(successors.len(), 2);

        assert_eq!(successors[1].r, [11.0, 3.0]);
        assert_eq!(successors[1].f, distance_between(from, [11.0, 3.0]));
        assert_eq!(
            successors[1].g,
            distance_between([11.0, 3.0], [9.75, 6.75]) + distance_between([9.75, 6.75], to)
        );
        assert_eq!(successors[1].polygon_from, 4);
        assert_eq!(successors[1].polygon_to, 2);
        assert_eq!(successors[1].i, [[10.0, 7.0], [9.75, 6.75]]);
        assert_eq!(successors[1].i_index, [11, 10]);
        assert_eq!(successors[1].path, vec![from]);

        assert_eq!(successors[0].r, from);
        assert_eq!(successors[0].f, 0.0);
        assert_eq!(successors[0].g, distance_between(from, to));
        assert_eq!(successors[0].polygon_from, 4);
        assert_eq!(successors[0].polygon_to, 2);
        assert_eq!(successors[0].i, [[9.75, 6.75], [7.0, 4.0]]);
        assert_eq!(successors[0].i_index, [11, 10]);
        assert_eq!(successors[0].path, Vec::<[f32; 2]>::new());

        assert_eq!(mesh.path(from, to).len, distance_between(from, to));
        assert_eq!(mesh.path(from, to).path, vec![to]);
    }

    #[test]
    fn paper_corner_right() {
        let mesh = mesh_from_paper();

        let from = [12.0, 0.0];
        let to = [13.0, 6.0];
        let search_node = SearchNode {
            path: vec![],
            r: from,
            i: [[11.0, 3.0], [7.0, 0.0]],
            i_index: [16, 15],
            polygon_from: mesh.point_in_polygon(from) as isize,
            polygon_to: 4,
            f: 0.0,
            g: distance_between(from, to),
        };
        let successors = dbg!(mesh.successors(search_node, to));
        assert_eq!(successors.len(), 3);

        assert_eq!(successors[0].r, [11.0, 3.0]);
        assert_eq!(successors[0].f, distance_between(from, [11.0, 3.0]));
        assert_eq!(
            successors[0].g,
            distance_between([11.0, 3.0], [11.0, 5.0]) + distance_between([11.0, 5.0], to)
        );
        assert_eq!(successors[0].polygon_from, 4);
        assert_eq!(successors[0].polygon_to, 6);
        assert_eq!(successors[0].i, [[11.0, 5.0], [10.0, 7.0]]);
        assert_eq!(successors[0].i_index, [17, 11]);
        assert_eq!(successors[0].path, vec![from]);

        assert_eq!(successors[1].r, [11.0, 3.0]);
        assert_eq!(successors[1].f, distance_between(from, [11.0, 3.0]));
        assert_eq!(
            successors[1].g,
            distance_between([11.0, 3.0], mirror(to, [[10.0, 7.0], [9.75, 6.75]]))
        );
        assert_eq!(successors[1].polygon_from, 4);
        assert_eq!(successors[1].polygon_to, 2);
        assert_eq!(successors[1].i, [[10.0, 7.0], [9.75, 6.75]]);
        assert_eq!(successors[1].i_index, [11, 10]);
        assert_eq!(successors[1].path, vec![from]);

        assert_eq!(successors[2].r, from);
        assert_eq!(successors[2].f, 0.0);
        assert_eq!(
            successors[2].g,
            distance_between(from, [9.75, 6.75])
                + distance_between([9.75, 6.75], mirror(to, [[9.75, 6.75], [7.0, 4.0]]))
        );
        assert_eq!(successors[2].polygon_from, 4);
        assert_eq!(successors[2].polygon_to, 2);
        assert_eq!(successors[2].i, [[9.75, 6.75], [7.0, 4.0]]);
        assert_eq!(successors[2].i_index, [11, 10]);
        assert_eq!(successors[2].path, Vec::<[f32; 2]>::new());

        assert_delta!(
            mesh.path(from, to).len,
            distance_between(from, [11.0, 3.0])
                + distance_between([11.0, 3.0], [11.0, 5.0])
                + distance_between([11.0, 5.0], to)
        );
        assert_eq!(mesh.path(from, to).path, vec![[11.0, 3.0], [11.0, 5.0], to]);
    }

    #[test]
    fn paper_corner_left() {
        let mesh = mesh_from_paper();

        let from = [12.0, 0.0];
        let to = [5.0, 3.0];
        let search_node = SearchNode {
            path: vec![],
            r: from,
            i: [[11.0, 3.0], [7.0, 0.0]],
            i_index: [16, 15],
            polygon_from: mesh.point_in_polygon(from) as isize,
            polygon_to: 4,
            f: 0.0,
            g: distance_between(from, to),
        };
        let successors = dbg!(mesh.successors(search_node, to));
        assert_eq!(successors.len(), 2);

        assert_eq!(successors[1].r, [11.0, 3.0]);
        assert_eq!(successors[1].f, distance_between(from, [11.0, 3.0]));
        assert_eq!(
            successors[1].g,
            distance_between([11.0, 3.0], [9.75, 6.75]) + distance_between([9.75, 6.75], to)
        );
        assert_eq!(successors[1].polygon_from, 4);
        assert_eq!(successors[1].polygon_to, 2);
        assert_eq!(successors[1].i, [[10.0, 7.0], [9.75, 6.75]]);
        assert_eq!(successors[1].i_index, [11, 10]);
        assert_eq!(successors[1].path, vec![from]);

        assert_eq!(successors[0].r, from);
        assert_eq!(successors[0].f, 0.0);
        assert_eq!(
            successors[0].g,
            distance_between(from, [7.0, 4.0]) + distance_between([7.0, 4.0], to)
        );
        assert_eq!(successors[0].polygon_from, 4);
        assert_eq!(successors[0].polygon_to, 2);
        assert_eq!(successors[0].i, [[9.75, 6.75], [7.0, 4.0]]);
        assert_eq!(successors[0].i_index, [11, 10]);
        assert_eq!(successors[0].path, Vec::<[f32; 2]>::new());

        assert_delta!(
            mesh.path(from, to).len,
            distance_between(from, [7.0, 4.0]) + distance_between([7.0, 4.0], to)
        );
        assert_eq!(mesh.path(from, to).path, vec![[7.0, 4.0], to]);
    }

    #[test]
    fn paper_corner_left_twice() {
        let mesh = mesh_from_paper();

        let from = [12.0, 0.0];
        let to = [3.0, 1.0];
        let search_node = SearchNode {
            path: vec![],
            r: from,
            i: [[11.0, 3.0], [7.0, 0.0]],
            i_index: [16, 15],
            polygon_from: mesh.point_in_polygon(from) as isize,
            polygon_to: 4,
            f: 0.0,
            g: distance_between(from, to),
        };
        let successors = dbg!(mesh.successors(search_node, to));
        assert_eq!(successors.len(), 2);

        assert_eq!(successors[1].r, [11.0, 3.0]);
        assert_eq!(successors[1].f, distance_between(from, [11.0, 3.0]));
        assert_eq!(
            successors[1].g,
            distance_between([11.0, 3.0], [9.75, 6.75]) + distance_between([9.75, 6.75], to)
        );
        assert_eq!(successors[1].polygon_from, 4);
        assert_eq!(successors[1].polygon_to, 2);
        assert_eq!(successors[1].i, [[10.0, 7.0], [9.75, 6.75]]);
        assert_eq!(successors[1].i_index, [11, 10]);
        assert_eq!(successors[1].path, vec![from]);

        assert_eq!(successors[0].r, from);
        assert_eq!(successors[0].f, 0.0);
        assert_eq!(
            successors[0].g,
            distance_between(from, [7.0, 4.0]) + distance_between([7.0, 4.0], to)
        );
        assert_eq!(successors[0].polygon_from, 4);
        assert_eq!(successors[0].polygon_to, 2);
        assert_eq!(successors[0].i, [[9.75, 6.75], [7.0, 4.0]]);
        assert_eq!(successors[0].i_index, [11, 10]);
        assert_eq!(successors[0].path, Vec::<[f32; 2]>::new());

        let successor = successors.into_iter().next().unwrap();
        let successors = dbg!(mesh.successors(successor, to));
        dbg!(&successors[0]);
        assert_eq!(successors.len(), 1);

        assert_delta!(
            mesh.path(from, to).len,
            distance_between(from, [7.0, 4.0])
                + distance_between([7.0, 4.0], [4.0, 2.0])
                + distance_between([4.0, 2.0], to)
        );

        assert_eq!(mesh.path(from, to).path, vec![[7.0, 4.0], [4.0, 2.0], to]);
    }

    #[test]
    fn edges_between_simple() {
        let mesh = mesh_from_paper();

        let from = [12.0, 0.0];
        let to = [3.0, 1.0];
        let search_node = SearchNode {
            path: vec![],
            r: from,
            i: [[11.0, 3.0], [7.0, 0.0]],
            i_index: [16, 15],
            polygon_from: mesh.point_in_polygon(from) as isize,
            polygon_to: 4,
            f: 0.0,
            g: distance_between(from, to),
        };

        let successors = mesh.edges_between(&search_node);

        for successor in &successors {
            println!("{:?}", successor);
        }

        println!("=========================");

        let search_node = SearchNode {
            path: vec![],
            r: from,
            i: [[9.75, 6.75], [7.0, 4.0]],
            i_index: [11, 10],
            polygon_from: 4,
            polygon_to: 2,
            f: 0.0,
            g: distance_between(from, to),
        };

        let successors = mesh.edges_between(&search_node);

        for successor in &successors {
            println!("{:?}", successor);
        }

        println!("=========================");

        let search_node = SearchNode {
            path: vec![],
            r: [11.0, 3.0],
            i: [[10.0, 7.0], [7.0, 4.0]],
            i_index: [11, 10],
            polygon_from: 4,
            polygon_to: 2,
            f: 0.0,
            g: distance_between(from, to),
        };

        let successors = mesh.edges_between(&search_node);

        for successor in &successors {
            println!("{:?}", successor);
        }

        // assert!(false);
    }

    #[test]
    fn edges_between_simple_u() {
        let mesh = mesh_u_grid();

        let search_node = SearchNode {
            path: vec![],
            r: [0.0, 0.0],
            i: [[1.0, 0.0], [1.0, 1.0]],
            i_index: [1, 5],
            polygon_from: 0,
            polygon_to: 1,
            f: 0.0,
            g: 1.0,
        };

        let successors = mesh.edges_between(&search_node);

        for successor in &successors {
            println!("{:?}", successor);
        }
        // assert!(false);
    }
}
