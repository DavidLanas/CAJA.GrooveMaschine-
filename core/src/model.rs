//! Modelo compartido entre el plugin, la interfaz y la tarea de fondo: sesión protegida por un
//! mutex (NUNCA lo toca el hilo de audio), calendario por triple buffer y estado en atómicos.

use crate::genres::GenreSet;
use crate::paths;
use crate::schedule::{Schedule, TripleBuffer};
use crate::session::{ParamSnap, Session};
use crate::shared::HostInfo;
use std::sync::{Arc, Mutex, RwLock};

pub struct Model {
    pub session: Mutex<Session>,
    pub schedule: TripleBuffer<Schedule>,
    pub host: HostInfo,
    /// Cadena JSON del estado de usuario: es el campo `#[persist]` del plugin.
    pub persist: Arc<Mutex<String>>,
    /// Nombres de género que usa el parámetro `Genero` para mostrarse.
    pub genre_names: Arc<RwLock<Vec<String>>>,
}

fn names_of(g: &GenreSet) -> Vec<String> {
    g.genres.iter().map(|x| x.name.clone()).collect()
}

impl Model {
    pub fn new(persist: Arc<Mutex<String>>, genre_names: Arc<RwLock<Vec<String>>>) -> Model {
        let s = Session::new(paths::load_genres(), paths::load_maps());
        *genre_names.write().unwrap_or_else(|e| e.into_inner()) = names_of(&s.genres);
        Model { session: Mutex::new(s), schedule: TripleBuffer::default(), host: HostInfo::new(), persist, genre_names }
    }

    pub fn with_session(session: Session, persist: Arc<Mutex<String>>) -> Model {
        let names = Arc::new(RwLock::new(names_of(&session.genres)));
        Model { session: Mutex::new(session), schedule: TripleBuffer::default(), host: HostInfo::new(), persist, genre_names: names }
    }

    /// Ejecuta `f` con la sesión bloqueada y después publica el calendario / guarda el estado si hizo falta.
    /// Hilos permitidos: principal, interfaz, tarea de fondo. Nunca el de audio.
    pub fn with<R>(&self, snap: ParamSnap, f: impl FnOnce(&mut Session) -> R) -> R {
        let mut s = self.session.lock().unwrap_or_else(|e| e.into_inner());
        self.pull_persisted(&mut s, snap);
        s.sync(snap);
        let r = f(&mut s);
        self.finish(&mut s);
        r
    }

    /// Solo sincroniza con los parámetros (interfaz cada fotograma, tarea de fondo, `initialize`).
    pub fn sync(&self, snap: ParamSnap) {
        self.with(snap, |_| ());
    }

    /// Si el host cargó un estado nuevo en el campo persistente, lo lee (rechazando el corrupto).
    fn pull_persisted(&self, s: &mut Session, snap: ParamSnap) {
        let cur = self.persist.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if cur != s.last_serialized {
            s.last_serialized = cur.clone();
            if !cur.is_empty() {
                if let Err(e) = s.load_state_json(&cur, snap) {
                    s.state_error = Some(format!("Estado guardado rechazado: {e}. Se usa el patrón generado."));
                    // regenera desde los parámetros en lugar de dejar un patrón a medias
                    s.state_dirty = true;
                }
            }
        }
    }

    fn finish(&self, s: &mut Session) {
        if s.sched_dirty {
            s.sched_dirty = false;
            let (loop_ticks, hits) = s.hits();
            self.schedule.publish(|sch| sch.fill(loop_ticks, &hits));
        }
        if s.state_dirty {
            s.state_dirty = false;
            let json = s.to_state_json();
            s.last_serialized = json.clone();
            *self.persist.lock().unwrap_or_else(|e| e.into_inner()) = json;
        }
    }

    /// Botón "Recargar": relee genres.json y los mapas; actualiza los nombres del parámetro.
    pub fn reload_files(&self, snap: ParamSnap) {
        self.with(snap, |s| {
            s.reload_files();
            *self.genre_names.write().unwrap_or_else(|e| e.into_inner()) = names_of(&s.genres);
        });
    }
}
