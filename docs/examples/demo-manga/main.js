// Demo manga: a B-Anime extension that serves placeholder pages for any search.
// Pages are photos from picsum.photos, seeded so each chapter always shows the same ones.

const CHAPTERS = 5;
const PAGES = 8;

class Provider {
  async search(query) {
    console.log("demo manga search for", query.query);
    return [{ id: "demo", title: "Demo manga (placeholder pages)" }];
  }

  async chapters(id) {
    return Array.from({ length: CHAPTERS }, (_, i) => ({ id: String(i + 1), number: i + 1, title: `Placeholder chapter ${i + 1}` }));
  }

  async pages(chapterId) {
    return {
      pages: Array.from({ length: PAGES }, (_, p) => ({
        url: `https://picsum.photos/seed/banime-${chapterId}-${p + 1}/900/1300`,
      })),
    };
  }
}
