
(function () {
  'use strict'

  $.get("/tags/json")
  .done(function( data ) {
    $.each(data, function () {
      if(this.deactivated) {return;}
      var newTag = $("body").find("#tag").clone();
      newTag.find(".card-header").append(this.name);
      if(this.color != null){
        newTag.find(".card-header").css('background-color', this.color);
      }
      newTag.find(".edit-tag").attr("href","/tags/"+this.id+"");
      newTag.find(".remove-tag").on('click', function(){
        $.ajax({
          url: '/tags/'+this.id,
          type: 'DELETE',
          success: function(result) {
            location.reload();
          }
      });
      }.bind(this)
    );
      newTag.show();
      $(newTag).appendTo("#tags");
    });
  });

})()
