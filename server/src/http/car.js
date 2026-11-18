(function () {
  var stages = document.querySelectorAll(".car-stage.has-render");
  Array.prototype.forEach.call(stages, function (stage) {
    var photo = stage.querySelector("img.car-photo");
    if (!photo) return;
    function show() {
      stage.classList.add("photo-ready");
    }
    function drop() {
      stage.classList.remove("photo-ready");
      stage.classList.remove("has-render");
      if (photo.parentNode) photo.parentNode.removeChild(photo);
    }
    photo.addEventListener("load", show);
    photo.addEventListener("error", drop);
    if (photo.complete) {
      if (photo.naturalWidth > 0) show();
      else drop();
    }
  });
})();
