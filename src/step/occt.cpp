// The STEP child's work through OpenCASCADE: read the file with its colors
// and assembly, then heal and mesh each part on a thread of its own, and hand
// it back at every place the assembly puts it. Reached from tessellate.rs
// through `tridi_step_tessellate`, the only symbol it exports.

#include <BRepBndLib.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <BRep_Tool.hxx>
#include <Bnd_Box.hxx>
#include <GeomLProp_SLProps.hxx>
#include <Message_ProgressRange.hxx>
#include <Poly_Triangulation.hxx>
#include <Quantity_Color.hxx>
#include <STEPCAFControl_Reader.hxx>
#include <STEPControl_Reader.hxx>
#include <Standard_Failure.hxx>
#include <Standard_Version.hxx>
#include <TDF_ChildIterator.hxx>
#include <TDF_LabelSequence.hxx>
#include <TDocStd_Document.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <XCAFApp_Application.hxx>
#include <XCAFDoc_ColorTool.hxx>
#include <XCAFDoc_DocumentTool.hxx>
#include <XCAFDoc_ShapeTool.hxx>
#if OCC_VERSION_HEX >= 0x070800
#include <ShapeProcess_ShapeContext.hxx>
#include <XSAlgo_ShapeProcessor.hxx>
#endif

#include <Precision.hxx>
#include <TopTools_DataMapOfShapeShape.hxx>

#include <algorithm>
#include <array>
#include <atomic>
#include <cmath>
#include <cstdint>
#include <mutex>
#include <optional>
#include <thread>
#include <unordered_map>
#include <vector>

namespace {

using Color = std::array<float, 3>;

// the color of a face the file gives none: negative, as no real color is
// (step/mod.rs's NONE)
constexpr Color NONE = {-1.0F, -1.0F, -1.0F};

// what tridi_step_tessellate returns
enum Status : int { OK = 0, NOT_STEP = 1, NO_SOLID = 2, WRITE = 3 };

using IdsFn = int (*)(void *, const uint64_t *, size_t);
using PartFn = int (*)(void *, uint64_t, const float *, size_t);

// A part to mesh once: its shape, its faces' own colors, and each place the
// assembly puts it with the color that instance gives it, if any.
struct Part {
    TopoDS_Shape shape;
    std::unordered_map<const TopoDS_TShape *, Color> faces;
    std::optional<Color> own;
    std::vector<std::pair<gp_Trsf, std::optional<Color>>> at;
};

std::optional<Color> color_of(const Handle(XCAFDoc_ColorTool) & tool, const TDF_Label &l) {
    Quantity_Color c;
    if (tool->GetColor(l, XCAFDoc_ColorSurf, c) || tool->GetColor(l, XCAFDoc_ColorGen, c)) {
        // OpenCASCADE keeps them linear; the file's values, which the viewer
        // takes, are sRGB
        double r, g, b;
        c.Values(r, g, b, Quantity_TOC_sRGB);
        return Color{float(r), float(g), float(b)};
    }
    return std::nullopt;
}

// The colors of a part's sub-shapes (a face, or a solid or shell for all its
// faces), by face.
void sub_colors(const Handle(XCAFDoc_ColorTool) & tool, const TDF_Label &part, Part &p) {
    for (TDF_ChildIterator it(part); it.More(); it.Next()) {
        const TDF_Label &sub = it.Value();
        auto c = color_of(tool, sub);
        if (!c) {
            continue;
        }
        TopoDS_Shape s = XCAFDoc_ShapeTool::GetShape(sub);
        for (TopExp_Explorer e(s, TopAbs_FACE); e.More(); e.Next()) {
            // a face's own color wins over its solid's: labels come in no set order
            auto [slot, fresh] = p.faces.try_emplace(e.Current().TShape().get(), *c);
            if (!fresh && s.ShapeType() == TopAbs_FACE) {
                slot->second = *c;
            }
        }
    }
}

// Every part under `l`, with the place and instance color the path to it
// gives; each part once, in the order first met (the ids the parent asks
// for again: the same file gives the same order).
void walk(const TDF_Label &l, const gp_Trsf &at, std::optional<Color> tint,
          const Handle(XCAFDoc_ColorTool) & colors, std::vector<TDF_Label> &seen, std::vector<Part> &parts) {
    if (XCAFDoc_ShapeTool::IsAssembly(l)) {
        TDF_LabelSequence comps;
        XCAFDoc_ShapeTool::GetComponents(l, comps);
        for (const TDF_Label &c : comps) {
            TDF_Label ref;
            if (!XCAFDoc_ShapeTool::GetReferredShape(c, ref)) {
                continue;
            }
            auto inner = color_of(colors, c);
            walk(ref, at * XCAFDoc_ShapeTool::GetLocation(c).Transformation(), inner ? inner : tint, colors, seen,
                 parts);
        }
        return;
    }
    TopoDS_Shape shape = XCAFDoc_ShapeTool::GetShape(l);
    if (shape.IsNull()) {
        return;
    }
    // ponytail: a linear search, a hashed map if an assembly has thousands of parts
    size_t at_part = std::find(seen.begin(), seen.end(), l) - seen.begin();
    if (at_part == seen.size()) {
        seen.push_back(l);
        Part p;
        p.shape = shape;
        p.own = color_of(colors, l);
        sub_colors(colors, l, p);
        parts.push_back(std::move(p));
    }
    parts[at_part].at.emplace_back(at, tint);
}

// What the healing did to the faces, for their colors: a face it replaced
// takes the color of the one it came from.
#if OCC_VERSION_HEX >= 0x070800
void follow(const Handle(ShapeProcess_ShapeContext) & ctx, Part &p) {
    if (ctx.IsNull()) {
        return;
    }
    std::unordered_map<const TopoDS_TShape *, Color> moved;
    for (TopTools_DataMapIteratorOfDataMapOfShapeShape it(ctx->Map()); it.More(); it.Next()) {
        auto c = p.faces.find(it.Key().TShape().get());
        if (c == p.faces.end()) {
            continue;
        }
        for (TopExp_Explorer e(it.Value(), TopAbs_FACE); e.More(); e.Next()) {
            moved.emplace(e.Current().TShape().get(), c->second);
        }
    }
    p.faces.merge(moved);
}
#endif

// A part's triangles in its own coordinates: position, normal, its face's
// color (or NONE), FLOATS per vertex; the instance colors come later.
struct Mesh {
    std::vector<float> v;
    // per triangle: has its face a color of its own
    std::vector<bool> own;
};

Mesh mesh(Part &p, bool coarse) {
    Bnd_Box box;
    BRepBndLib::Add(p.shape, box);
    double diag = box.IsVoid() ? 1.0 : std::sqrt(box.SquareExtent());
    // ponytail: shares of the part's size; a setting if a file needs another
    double deflection = std::max(diag * (coarse ? 0.005 : 0.001), Precision::Confusion());
    BRepMesh_IncrementalMesh(p.shape, deflection, Standard_False, coarse ? 0.8 : 0.4, Standard_False);
    Mesh m;
    for (TopExp_Explorer e(p.shape, TopAbs_FACE); e.More(); e.Next()) {
        const TopoDS_Face &face = TopoDS::Face(e.Current());
        TopLoc_Location loc;
        Handle(Poly_Triangulation) tri = BRep_Tool::Triangulation(face, loc);
        if (tri.IsNull()) {
            continue;
        }
        // the surface's normals at the nodes, in the nodes' frame; the
        // triangle's own where the surface has none (an apex)
        TopLoc_Location at_surface;
        Handle(Geom_Surface) surface = BRep_Tool::Surface(face, at_surface);
        std::vector<std::optional<gp_Dir>> normals(tri->NbNodes() + 1);
        if (tri->HasUVNodes() && !surface.IsNull()) {
            // the surface's frame is the face's less the nodes' own
            gp_Trsf to_nodes = loc.Transformation().Inverted().Multiplied(at_surface.Transformation());
            for (int k = 1; k <= tri->NbNodes(); k++) {
                gp_Pnt2d uv = tri->UVNode(k);
                GeomLProp_SLProps props(surface, uv.X(), uv.Y(), 1, Precision::Confusion());
                if (props.IsNormalDefined()) {
                    normals[k] = props.Normal().Transformed(to_nodes);
                }
            }
        }
        const gp_Trsf &t = loc.Transformation();
        bool reversed = face.Orientation() == TopAbs_REVERSED;
        auto c = p.faces.find(face.TShape().get());
        Color color = c != p.faces.end() ? c->second : NONE;
        for (int i = 1; i <= tri->NbTriangles(); i++) {
            int a, b, cc;
            tri->Triangle(i).Get(a, b, cc);
            if (reversed) {
                std::swap(b, cc);
            }
            gp_Vec ab(tri->Node(a), tri->Node(b)), ac(tri->Node(a), tri->Node(cc));
            gp_Vec cross = ab.Crossed(ac);
            // a degenerate triangle shows nothing, and has no normal
            if (cross.SquareMagnitude() <= 0.0) {
                continue;
            }
            for (int k : {a, b, cc}) {
                gp_Pnt q = tri->Node(k).Transformed(t);
                // the surface's points one way, the face may go the other;
                // the winding already goes the face's way
                const std::optional<gp_Dir> &on_surface = normals[k];
                gp_Dir n = on_surface ? (reversed ? on_surface->Reversed() : *on_surface) : gp_Dir(cross);
                n.Transform(t);
                m.v.insert(m.v.end(), {float(q.X()), float(q.Y()), float(q.Z()), float(n.X()), float(n.Y()),
                                       float(n.Z()), color[0], color[1], color[2]});
            }
            m.own.push_back(c != p.faces.end());
        }
    }
    return m;
}

constexpr size_t FLOATS = 9;

// The part's triangles at each of its places, with the color each vertex
// shows: its face's own, else the instance's, else the part's, else NONE.
std::vector<float> place(const Mesh &m, const Part &p) {
    std::vector<float> out;
    out.reserve(m.v.size() * p.at.size());
    for (const auto &[t, tint] : p.at) {
        std::optional<Color> fill = tint ? tint : p.own;
        for (size_t i = 0; i + FLOATS <= m.v.size(); i += FLOATS) {
            gp_Pnt q(m.v[i], m.v[i + 1], m.v[i + 2]);
            gp_Dir n(m.v[i + 3], m.v[i + 4], m.v[i + 5]);
            q.Transform(t);
            n.Transform(t);
            bool own = m.own[i / (3 * FLOATS)];
            Color c = own ? Color{m.v[i + 6], m.v[i + 7], m.v[i + 8]} : fill.value_or(NONE);
            out.insert(out.end(), {float(q.X()), float(q.Y()), float(q.Z()), float(n.X()), float(n.Y()),
                                   float(n.Z()), c[0], c[1], c[2]});
        }
    }
    return out;
}

// STEPControl_Reader keeps the default healing to itself
struct Healing : STEPControl_Reader {
#if OCC_VERSION_HEX >= 0x070800
    using STEPControl_Reader::GetDefaultShapeFixParameters;
    using STEPControl_Reader::GetDefaultShapeProcessFlags;
#endif
};

} // namespace

// `ids` first gets the parts to do: the file's, or those of them in `only`
// if it isn't empty; then `part` gets each one's id and triangles at every
// place the assembly uses it, from several threads at once, as soon as it's
// done (empty if it gave none). A nonzero return from either stops it.
extern "C" int tridi_step_tessellate(const char *path, int coarse, const uint64_t *only, size_t n_only, void *ctx,
                                     IdsFn ids, PartFn part) {
    std::vector<Part> parts;
    try {
        STEPCAFControl_Reader reader;
        reader.SetColorMode(true);
        reader.SetNameMode(false);
        reader.SetLayerMode(false);
        if (reader.ReadFile(path) != IFSelect_RetDone) {
            return NOT_STEP;
        }
#if OCC_VERSION_HEX >= 0x070800
        // the healing, the bulk of the reading, is done below a part at a
        // time on every core; before 7.8 the reader does it, on one. After
        // ReadFile, which sets them back, and on both readers
        reader.SetShapeProcessFlags(ShapeProcess::OperationsFlags());
        reader.ChangeReader().SetShapeProcessFlags(ShapeProcess::OperationsFlags());
#endif
        Handle(TDocStd_Document) doc;
        XCAFApp_Application::GetApplication()->NewDocument("MDTV-XCAF", doc);
        if (!reader.Transfer(doc)) {
            return NO_SOLID;
        }
        Handle(XCAFDoc_ShapeTool) shapes = XCAFDoc_DocumentTool::ShapeTool(doc->Main());
        Handle(XCAFDoc_ColorTool) colors = XCAFDoc_DocumentTool::ColorTool(doc->Main());
        TDF_LabelSequence roots;
        shapes->GetFreeShapes(roots);
        std::vector<TDF_Label> seen;
        for (const TDF_Label &r : roots) {
            walk(r, XCAFDoc_ShapeTool::GetLocation(r).Transformation(), color_of(colors, r), colors, seen, parts);
        }
    } catch (...) {
        return NOT_STEP;
    }
    std::vector<uint64_t> todo;
    for (uint64_t id = 1; id <= parts.size(); id++) {
        if (n_only == 0 || std::find(only, only + n_only, id) != only + n_only) {
            todo.push_back(id);
        }
    }
    if (todo.empty()) {
        return NO_SOLID;
    }
    if (ids(ctx, todo.data(), todo.size()) != 0) {
        return WRITE;
    }
#if OCC_VERSION_HEX >= 0x070800
    Healing defaults;
    auto params = defaults.GetDefaultShapeFixParameters();
    auto flags = defaults.GetDefaultShapeProcessFlags();
#endif
    std::atomic<size_t> next{0};
    std::atomic<bool> any{false}, failed{false};
    auto work = [&] {
        for (size_t i; !failed && (i = next++) < todo.size();) {
            uint64_t id = todo[i];
            Part &p = parts[id - 1];
            std::vector<float> v;
            // a part OpenCASCADE throws on is lost, like one it can't mesh;
            // nothing may cross into Rust
            try {
#if OCC_VERSION_HEX >= 0x070800
                XSAlgo_ShapeProcessor healer(params);
                p.shape = healer.ProcessShape(p.shape, flags, Message_ProgressRange());
                follow(healer.GetContext(), p);
#endif
                v = place(mesh(p, coarse != 0), p);
            } catch (...) {
                v.clear();
            }
            any = any || !v.empty();
            if (part(ctx, id, v.data(), v.size()) != 0) {
                failed = true;
            }
        }
    };
    size_t n = std::min<size_t>(std::max(1U, std::thread::hardware_concurrency()), todo.size());
    std::vector<std::thread> threads;
    for (size_t k = 0; k < n; k++) {
        threads.emplace_back(work);
    }
    for (auto &t : threads) {
        t.join();
    }
    if (failed) {
        return WRITE;
    }
    return any ? OK : NO_SOLID;
}
